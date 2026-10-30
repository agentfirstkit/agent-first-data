#!/usr/bin/env bash
# Sourceable Bash helpers for writing AFDATA-style command-line scripts.
# This file deliberately changes no shell options and performs no work when sourced.

if [ "${_AFDATA_SH_LOADED:-0}" = "1" ]; then
  # `return` handles repeated sourcing; `exit` only covers deliberate execution
  # with the guard variable pre-set.
  # shellcheck disable=SC2317
  return 0 2>/dev/null || exit 0
fi
_AFDATA_SH_LOADED=1
# Public API marker read by callers that pin a supported helper surface.
# shellcheck disable=SC2034
AFDATA_BASH_API_VERSION=2

_AFDATA_ARGS_OPTION_VARS=()
_AFDATA_ARGS_OPTION_FLAGS=()
_AFDATA_ARGS_OPTION_VALUE_NAMES=()
_AFDATA_ARGS_OPTION_DESCRIPTIONS=()
_AFDATA_ARGS_OPTION_DEFAULTS=()
_AFDATA_ARGS_FLAG_VARS=()
_AFDATA_ARGS_FLAG_FLAGS=()
_AFDATA_ARGS_FLAG_DESCRIPTIONS=()
_AFDATA_ARGS_POSITIONAL_VARS=()
_AFDATA_ARGS_POSITIONAL_NAMES=()
_AFDATA_ARGS_POSITIONAL_DESCRIPTIONS=()
_AFDATA_ARGS_POSITIONAL_MODES=()
_AFDATA_ARGS_REST_NAME=""
_AFDATA_ARGS_REST_DESCRIPTION=""
AFDATA_ARGS_REST=()

afdata_cli() {
  local afdata_bin="${AFDATA_BIN:-afdata}"
  command "$afdata_bin" "$@"
}

# Usage: _afdata_emit KIND [OPTION...] -- [POSITIONAL...]
# Callers put `--` before the positionals they forward, so a message such as
# "-5 files" reaches `emit` as a value rather than being parsed as a flag.
_afdata_emit() {
  local kind="$1"
  shift
  afdata_cli \
    emit "$kind" \
    --output "${AFDATA_OUTPUT:-json}" \
    --output-to "${AFDATA_OUTPUT_TO:-split}" \
    "$@"
}

afdata_log() {
  if [ "$#" -ne 2 ]; then
    _afdata_function_error \
      "afdata_log requires LEVEL and MESSAGE" \
      "usage: afdata_log <debug|info|warn|error> <MESSAGE>"
    return 2
  fi
  _afdata_emit log -- "$1" "$2"
}

afdata_result() {
  # A lone `--json` is a payload left out, not a message spelled `--json`.
  if ! { [ "$#" -eq 1 ] && [ "$1" != "--json" ]; } \
    && ! { [ "$#" -eq 2 ] && [ "$1" = "--json" ]; }; then
    _afdata_function_error \
      "afdata_result requires MESSAGE or --json JSON" \
      "usage: afdata_result <MESSAGE> | --json <JSON>"
    return 2
  fi
  # A child invoked through afdata_call participates in its parent's finite
  # event stream. The outermost script owns the unique terminal result, so a
  # successful child completion is diagnostic rather than terminal.
  if [ "${_AFDATA_BASH_CHILD:-0}" = "1" ]; then
    if [ "$#" -eq 2 ]; then
      # Redact the structured payload before demoting it to diagnostic text.
      # Never interpolate the caller's raw JSON into a log message.
      local child_result
      # Split, not stdout: a rejected payload's error event must reach stderr
      # rather than vanish into the discarded capture.
      child_result="$(afdata_cli emit result --json="$2" --output json --output-to split)" || return $?
      afdata_log info "$child_result"
    else
      afdata_log info "$1"
    fi
    return $?
  fi
  if [ "$#" -eq 2 ]; then
    _afdata_emit result --json="$2"
  else
    _afdata_emit result -- "$1"
  fi
}

afdata_error() {
  if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    _afdata_function_error \
      "afdata_error requires CODE, MESSAGE, and an optional HINT" \
      "usage: afdata_error <CODE> <MESSAGE> [HINT]"
    return 2
  fi
  if [ "$#" -eq 3 ]; then
    _afdata_emit error --hint="$3" -- "$1" "$2"
  else
    _afdata_emit error -- "$1" "$2"
  fi
}

_afdata_function_error() {
  local message="$1"
  local hint="$2"
  if ! _afdata_emit error --hint="$hint" -- cli_error "$message"; then
    :
  fi
  return 2
}

afdata_config_get() {
  if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    _afdata_function_error \
      "afdata_config_get requires FILE, KEY, and an optional DEFAULT" \
      "usage: afdata_config_get <FILE> <KEY> [DEFAULT]"
    return 2
  fi
  if [ "$#" -eq 3 ]; then
    afdata_cli value "$1" "$2" --default "$3"
  else
    afdata_cli value "$1" "$2"
  fi
}

# Remove one path, but only after `guard` has approved it.
#
# Exists because the obvious spelling cannot be made to fail loudly:
#
#     rm -rf -- "$(afdata guard tmp_path "$dir")"    # do not write this
#
# In an argument position the guard's exit status is discarded, and `rm -f` is
# defined to exit 0 when its operand names nothing — so a rejected value leaves
# the target untouched (which is the point of the guard) while the script runs
# on as though the removal had succeeded. That is not a platform quirk to route
# around: `-f` suppressing "it wasn't there" is what `-f` means.
#
# As a command, this function's exit status *is* the guard's, so `set -e` stops
# on a rejection. The verb stays in Bash; afdata only ever reads and validates.
#
# TYPE and any --under ROOT are forwarded to `guard` untouched, so the call
# reads exactly like the guard it wraps and this kit never has a second opinion
# about the vocabulary:
#
#     afdata_remove tmp_path "$work_dir"
#     afdata_remove path "$out_dir/$name" --under "$out_dir"
#
# Removing a path that does not exist succeeds, matching `rm -rf`, so repeated
# cleanup is idempotent. Callers that clean up several paths in one EXIT trap
# should decide explicitly what a rejection means: under `set -e` the first
# failure aborts the trap and later paths are never reached, so append `|| :`
# to each call when every path must be attempted.
afdata_remove() {
  if [ "$#" -lt 2 ]; then
    _afdata_function_error \
      "afdata_remove requires TYPE and VALUE" \
      "usage: afdata_remove <path|tmp_path|cwd_path> <VALUE> [--under <ROOT>]"
    return 2
  fi
  # Declared separately: `local target="$(...)"` would make `local` the command
  # whose status the shell sees, masking the very exit code this exists to keep.
  local _afdata_internal_target
  _afdata_internal_target="$(afdata_cli guard "$@")" || return
  rm -rf -- "$_afdata_internal_target"
}

# Name the platform's own trash entry point, or fail. Never invents one, and
# never falls back to a real delete: the whole value of the verb below is that
# it is reversible, and a silent downgrade would be the exact shape this kit
# exists to remove — a failure wearing success's clothes.
#
# Windows is decided first and by the shell it is running under, not by what is
# on PATH. It ships no trash command at all, so its entry point is PowerShell
# driving the same shell operation Explorer performs; and an MSYS distribution
# may well carry `trash-put`, which would write a freedesktop trash directory
# under $HOME that Explorer neither lists nor restores from. A trash can nobody
# can open is not the reversibility this verb promises, so the Unix entries are
# not consulted there.
_afdata_trash_command() {
  case "${OSTYPE:-}" in
    msys | cygwin | win32)
      if command -v powershell >/dev/null 2>&1; then
        printf 'powershell'
      elif command -v pwsh >/dev/null 2>&1; then
        printf 'pwsh'
      else
        return 1
      fi
      return 0
      ;;
  esac
  if command -v trash >/dev/null 2>&1; then
    printf 'trash'
  elif command -v gio >/dev/null 2>&1; then
    printf 'gio'
  elif command -v trash-put >/dev/null 2>&1; then
    printf 'trash-put'
  else
    return 1
  fi
}

# The Windows entry point, which had to be assembled by hand rather than named.
#
# Windows ships no trash command, and the obvious PowerShell recipe is actively
# wrong: `Microsoft.VisualBasic.FileIO.FileSystem.DeleteFile(...,
# SendToRecycleBin)` was measured on Windows 11 deleting outright — nothing
# reached the Recycle Bin at all — in precisely the sessions a script runs in,
# where no desktop exists. It reports success while doing the irreversible
# thing, which is the one failure this verb exists to prevent, so it is not used
# here. IFileOperation is the interface Explorer itself drives, and it recycles
# from a session with no desktop.
#
# Nothing about the Recycle Bin is reimplemented: placement, naming collisions,
# expiry and the "put back" record stay the shell's contract, exactly as they
# stay the desktop's on the other platforms.
#
# What Windows provides no call for is *failing* when it cannot recycle. Every
# route that cannot reach the bin deletes the entry permanently and reports
# success — a UNC path, a network drive, the per-volume NukeOnDelete switch, the
# NoRecycleFiles policy, and an entry larger than the bin's quota, each measured
# on a real host rather than reasoned about. So each is decided before anything
# is touched. `trash`, `gio trash` and `trash-put` fail closed by themselves;
# this is the same guarantee, assembled out of the parts Windows does give.
#
# Exit codes: 10 refused and nothing touched; 11 the operation failed and the
# entry is as it was; 12 the entry is gone but never reached the bin, which
# means the guarantee broke and saying so loudly is all that is left to do.
#
# The script is handed over as a file rather than on stdin, and the operand
# travels in the environment so that neither shell's quoting rules have to be
# reasoned about for a path that will routinely hold spaces and backslashes.
# `-Command -` was measured doing the one thing this verb may not do: PowerShell
# executes a piped script as it reads it, and on reaching the end it silently
# discarded the trailing statements — the recycle among them — and exited 0.
# A script that cannot be parsed in full has to fail loudly, which is what
# `-File` does, and the token below covers anything that manages not to.
_afdata_trash_windows() {
  local _afdata_internal_dir _afdata_internal_script
  local _afdata_internal_native _afdata_internal_status
  _afdata_internal_dir="$(mktemp -d)" || return 1
  _afdata_internal_script="$_afdata_internal_dir/trash.ps1"
  cat > "$_afdata_internal_script" <<'AFDATA_TRASH_POWERSHELL'
$ErrorActionPreference = 'Stop'
$target = $env:AFDATA_TRASH_TARGET

# stdout carries the one line a caller turns into an AFDATA error; the exit code
# says which error it is.
function Deny([int]$code, [string]$reason) {
  [Console]::Out.Write($reason)
  exit $code
}

# `guard` prints what canonicalization produces, and on Windows that carries the
# `\\?\` extended-length prefix. It is a genuine path — the shell's file tests,
# `rm` and .NET all accept it — but its leading backslashes read as UNC, and the
# shell's own item API does not take the prefixed form. So it is unwrapped here,
# before anything looks at the root. `\\?\UNC\server\share` is the real UNC
# spelling wearing the same prefix, and stays refused.
if ($target.StartsWith('\\?\')) {
  if ($target.StartsWith('\\?\UNC\')) {
    Deny 10 "a UNC path has no Recycle Bin: $target"
  }
  $target = $target.Substring(4)
}

$root = [System.IO.Path]::GetPathRoot($target)
if (-not $root -or $root.StartsWith('\\')) {
  Deny 10 "a UNC path has no Recycle Bin: $target"
}
$drive = New-Object System.IO.DriveInfo $root
if ($drive.DriveType -ne [System.IO.DriveType]::Fixed) {
  Deny 10 "$root is a $($drive.DriveType) drive, and only a fixed drive recycles"
}
foreach ($hive in 'HKCU:', 'HKLM:') {
  $policy = Get-ItemProperty -ErrorAction SilentlyContinue -Name NoRecycleFiles `
    -Path "$hive\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer"
  if ($policy -and $policy.NoRecycleFiles -eq 1) {
    Deny 10 'the NoRecycleFiles policy switches the Recycle Bin off on this host'
  }
}

# The per-volume settings are keyed by the volume's GUID, so a GUID that will
# not resolve means they cannot be read — and a guarantee that cannot be checked
# is refused rather than assumed.
$volumeId = $null
try { $volumeId = (Get-Volume -DriveLetter $root.Substring(0, 1)).UniqueId } catch {
  try {
    $volumeId = (Get-CimInstance -ErrorAction Stop -ClassName Win32_Volume `
      -Filter ("DriveLetter='{0}:'" -f $root.Substring(0, 1))).DeviceID
  } catch {}
}
if (-not ($volumeId -match '(\{[0-9a-fA-F-]+\})')) {
  Deny 10 "cannot identify the volume behind $root, so its Recycle Bin settings cannot be read"
}
$settings = Get-ItemProperty -ErrorAction SilentlyContinue `
  -Path ('HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\' + $Matches[1])
if ($settings -and $settings.NukeOnDelete -eq 1) {
  Deny 10 "the Recycle Bin is switched off for $root (NukeOnDelete)"
}

# An entry bigger than the bin's quota is deleted outright, so it is measured
# first. The walk stops the moment the limit is passed, and a subtree that
# cannot be read is left for the operation itself to fail on.
if ($settings -and $null -ne $settings.MaxCapacity) {
  $limit = [int64]$settings.MaxCapacity * 1MB
  $size = 0
  try {
    if (Test-Path -LiteralPath $target -PathType Container) {
      foreach ($file in [System.IO.Directory]::EnumerateFiles($target, '*', [System.IO.SearchOption]::AllDirectories)) {
        $size += (New-Object System.IO.FileInfo $file).Length
        if ($size -gt $limit) { break }
      }
    } else {
      $size = (New-Object System.IO.FileInfo $target).Length
    }
  } catch {}
  if ($size -gt $limit) {
    # A quota of zero is a real setting and means the bin holds nothing at all —
    # measured deleting outright rather than recycling — so it is refused like
    # any other oversize entry, but saying "0MB against a 0MB limit" would read
    # as a bug in this check rather than as the volume's configuration.
    if ($limit -eq 0) {
      Deny 10 "the Recycle Bin on $root is set to hold nothing (MaxCapacity is 0)"
    }
    Deny 10 ("$target is larger than the Recycle Bin on $root will hold (" +
      [int64]($size / 1MB) + "MB against a " + $settings.MaxCapacity + "MB limit)")
  }
}

try {
  Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace Afdata
{
    [ComImport, Guid("43826D1E-E718-42EE-BC55-A1E261C37BFE"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IShellItem
    {
        void BindToHandler(IntPtr pbc, ref Guid bhid, ref Guid riid, out IntPtr ppv);
        void GetParent(out IShellItem ppsi);
        void GetDisplayName(uint sigdnName, out IntPtr ppszName);
        void GetAttributes(uint sfgaoMask, out uint psfgaoAttribs);
        void Compare(IShellItem psi, uint hint, out int piOrder);
    }

    // Every method ahead of DeleteItem is declared even though none is called:
    // the declaration order is the vtable, and a shorter interface would call
    // whatever happens to sit in DeleteItem's slot.
    [ComImport, Guid("947aab5f-0a5c-4c13-b4d6-4bf7836fc9f8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IFileOperation
    {
        void Advise(IntPtr sink, out uint cookie);
        void Unadvise(uint cookie);
        void SetOperationFlags(uint flags);
        void SetProgressMessage([MarshalAs(UnmanagedType.LPWStr)] string message);
        void SetProgressDialog(IntPtr popd);
        void SetProperties(IntPtr array);
        void SetOwnerWindow(IntPtr owner);
        void ApplyPropertiesToItem(IShellItem item);
        void ApplyPropertiesToItems(object items);
        void RenameItem(IShellItem item, [MarshalAs(UnmanagedType.LPWStr)] string newName, IntPtr sink);
        void RenameItems(object items, [MarshalAs(UnmanagedType.LPWStr)] string newName);
        void MoveItem(IShellItem item, IShellItem destination, [MarshalAs(UnmanagedType.LPWStr)] string newName, IntPtr sink);
        void MoveItems(object items, IShellItem destination);
        void CopyItem(IShellItem item, IShellItem destination, [MarshalAs(UnmanagedType.LPWStr)] string copyName, IntPtr sink);
        void CopyItems(object items, IShellItem destination);
        void DeleteItem(IShellItem item, IntPtr sink);
        void DeleteItems(object items);
        void NewItem(IShellItem destination, uint attributes, [MarshalAs(UnmanagedType.LPWStr)] string name, [MarshalAs(UnmanagedType.LPWStr)] string templateName, IntPtr sink);
        void PerformOperations();
        void GetAnyOperationsAborted([MarshalAs(UnmanagedType.Bool)] out bool aborted);
    }

    public static class Trash
    {
        [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
        private static extern void SHCreateItemFromParsingName(
            [MarshalAs(UnmanagedType.LPWStr)] string path, IntPtr bindContext, ref Guid riid,
            [MarshalAs(UnmanagedType.Interface)] out IShellItem item);

        public static bool Recycle(string path, uint flags)
        {
            IFileOperation operation = (IFileOperation)Activator.CreateInstance(
                Type.GetTypeFromCLSID(new Guid("3ad05575-8857-4850-9277-11b85bdb8e09")));
            operation.SetOperationFlags(flags);
            Guid itemId = new Guid("43826D1E-E718-42EE-BC55-A1E261C37BFE");
            IShellItem item;
            SHCreateItemFromParsingName(path, IntPtr.Zero, ref itemId, out item);
            operation.DeleteItem(item, IntPtr.Zero);
            operation.PerformOperations();
            bool aborted;
            operation.GetAnyOperationsAborted(out aborted);
            return !aborted;
        }
    }
}
'@
} catch {
  Deny 10 'this PowerShell cannot compile the shell interop, so it cannot reach the Recycle Bin'
}

# Whether the entry really went to the bin is settled by the bin's own directory
# on this volume gaining something — the same evidence a person restoring the
# entry would go by, and independent of anything the API chooses to report.
$sid = ([System.Security.Principal.WindowsIdentity]::GetCurrent()).User.Value
$binDir = Join-Path (Join-Path $root '$Recycle.Bin') $sid
function Get-BinEntryCount {
  if (Test-Path -LiteralPath $binDir) {
    return @(Get-ChildItem -LiteralPath $binDir -Force -ErrorAction SilentlyContinue).Count
  }
  return 0
}
$before = Get-BinEntryCount

# FOF_NO_UI keeps a dialog from ever being drawn, which matters most where there
# is no desktop to draw one on; FOF_ALLOWUNDO is what asks for the bin.
if (-not [Afdata.Trash]::Recycle($target, (0x0614 -bor 0x0040))) {
  [Console]::Error.Write("the shell aborted trashing $target, which has been left alone")
  exit 11
}
if (Test-Path -LiteralPath $target) {
  [Console]::Error.Write("the shell reported no error, but $target is still on disk")
  exit 11
}
if ((Get-BinEntryCount) -le $before) {
  Deny 12 "$target was deleted, but nothing arrived in the Recycle Bin on $root"
}
# Printed as the last act of the script, so a caller can tell "ran to the end"
# from "stopped somewhere and exited 0".
[Console]::Out.Write('AFDATA-TRASH-OK')
AFDATA_TRASH_POWERSHELL
  # PowerShell reads a native path; the kit is running under a shell that hands
  # out POSIX ones.
  _afdata_internal_native="$(cygpath -w "$_afdata_internal_script")" || {
    rm -rf -- "$_afdata_internal_dir"
    return 1
  }
  _afdata_internal_status=0
  AFDATA_TRASH_TARGET="$2" command "$1" \
    -NoProfile -NonInteractive -ExecutionPolicy Bypass \
    -File "$_afdata_internal_native" || _afdata_internal_status=$?
  # This kit's own scratch file, created by mktemp two lines above and named by
  # nothing the caller supplied, so it is removed rather than routed through the
  # guard that exists for operands a caller can influence.
  rm -rf -- "$_afdata_internal_dir"
  return "$_afdata_internal_status"
}

# Move one guarded path to the platform's trash instead of deleting it.
#
# Same call shape as afdata_remove, and for the same reason: the guard's exit
# status lands in this function's, not in an argument position. Which of the two
# to use is the caller's decision, made in the source rather than guessed at
# runtime — build output and mktemp scratch belong to afdata_remove, a person's
# own files to this. Do not reach for it to clean temporary directories: they
# are meant to disappear, and trashing every build's scratch space fills the
# trash by the gigabyte with things nobody will ever restore.
#
# Where a trash exists, the system owns it — expiry, restore, and the "put back"
# metadata are the desktop's job, not this kit's, which is what keeps the
# reversible verb free of any state afdata would have to maintain.
#
# Where none exists — a container, a CI runner, a headless host — this fails
# with `trash_unavailable` rather than deleting. A caller that must proceed
# there wants afdata_remove and should say so. The same code covers a host that
# has a trash which this particular target cannot reach, because the caller's
# answer to both is the same one.
#
# Removing a path that is already gone succeeds, matching afdata_remove, so
# repeated cleanup stays idempotent.
afdata_trash() {
  if [ "$#" -lt 2 ]; then
    _afdata_function_error \
      "afdata_trash requires TYPE and VALUE" \
      "usage: afdata_trash <path|tmp_path|cwd_path> <VALUE> [--under <ROOT>]"
    return 2
  fi

  local _afdata_internal_target _afdata_internal_trash
  local _afdata_internal_reason _afdata_internal_status
  _afdata_internal_target="$(afdata_cli guard "$@")" || return

  # `-L` as well as `-e`: a dangling symlink is a real entry to trash even
  # though `-e` follows the link and reports it missing.
  if [ ! -e "$_afdata_internal_target" ] && [ ! -L "$_afdata_internal_target" ]; then
    return 0
  fi

  if ! _afdata_internal_trash="$(_afdata_trash_command)"; then
    afdata_error trash_unavailable \
      "no trash is available on this host" \
      "install one, or use afdata_remove when deletion is what the caller means" || :
    return 1
  fi

  case "$_afdata_internal_trash" in
    powershell | pwsh)
      # Exit 10 is the Windows entry point refusing, and it means what finding
      # no entry point at all means: nothing was touched, and this target cannot
      # be made reversible. It carries its own reason because "which host" is
      # not enough to act on when the answer turns on which drive the target
      # sits on. Exit 12 is the one outcome that is not interchangeable with it
      # — the entry is gone and did not reach the bin — so it gets a code of its
      # own rather than being folded into a refusal that promises the opposite.
      _afdata_internal_status=0
      _afdata_internal_reason="$(_afdata_trash_windows \
        "$_afdata_internal_trash" "$_afdata_internal_target")" \
        || _afdata_internal_status=$?
      case "$_afdata_internal_status" in
        0)
          # Exit 0 is not taken as success on its own. A PowerShell handed a
          # script it cannot parse to the end was measured running the part it
          # had, discarding the rest and exiting 0 — so the entry point prints a
          # token as its last act, and a missing token means the run stopped
          # somewhere unknown rather than that the target was trashed.
          if [ "$_afdata_internal_reason" != "AFDATA-TRASH-OK" ]; then
            afdata_error trash_incomplete \
              "the trash entry point exited 0 without running to completion" \
              "check whether $_afdata_internal_target still exists before retrying" || :
            return 1
          fi
          return 0
          ;;
        10)
          afdata_error trash_unavailable \
            "${_afdata_internal_reason:-no trash is available for this target}" \
            "use afdata_remove when deletion is what the caller means" || :
          return 1
          ;;
        12)
          afdata_error trash_not_reversible \
            "${_afdata_internal_reason:-the target was deleted without reaching the trash}" \
            "the entry is gone and cannot be restored from the trash" || :
          return 1
          ;;
      esac
      return "$_afdata_internal_status"
      ;;
    gio) command gio trash -- "$_afdata_internal_target" ;;
    *) command "$_afdata_internal_trash" -- "$_afdata_internal_target" ;;
  esac
}

# Invoke another executable that uses this Bash kit while keeping terminal
# ownership in the current script. The child keeps live AFDATA logs and errors;
# only its successful afdata_result is converted to an info log. Use afdata_run
# instead for raw programs or children that do not load this library.
afdata_call() {
  if [ "${1:-}" = "--" ]; then
    shift
  fi
  if [ "$#" -eq 0 ]; then
    _afdata_function_error \
      "afdata_call requires a command" \
      "usage: afdata_call [--] <AFDATA_BASH_COMMAND> [ARG ...]"
    return 2
  fi
  _AFDATA_BASH_CHILD=1 "$@"
}

# Run a child process in passthrough mode by default. --quiet buffers combined
# output, discards it on success, and replays it on stderr on failure. Only the
# wrapper's start/completion messages are AFDATA log events; a failure is a
# terminal child_process_failed error. Arguments are intentionally not logged
# because they may contain secrets.
afdata_run() {
  local _afdata_internal_quiet=false
  if [ "${1:-}" = "--quiet" ]; then
    _afdata_internal_quiet=true
    shift
  fi
  if [ "${1:-}" = "--" ]; then
    shift
  fi
  if [ "$#" -eq 0 ]; then
    _afdata_function_error \
      "afdata_run requires a command" \
      "usage: afdata_run [--quiet] [--] <COMMAND> [ARG ...]"
    return 2
  fi

  # Reserved locals keep an invoked shell function's dynamic scope unchanged.
  local _afdata_internal_command_name="${1##*/}"
  local _afdata_internal_child_status
  local _afdata_internal_output=""
  afdata_log info "Running ${_afdata_internal_command_name}"

  if [ "$_afdata_internal_quiet" = true ]; then
    # Buffer the child's combined output in memory rather than a temp file, so
    # an interrupted or signalled script leaves nothing to clean up and no
    # caller trap is touched. Command substitution runs the child in a subshell,
    # which is fine for the noninteractive programs (cargo, npm, …) --quiet
    # exists for.
    if _afdata_internal_output="$("$@" 2>&1)"; then
      afdata_log info "${_afdata_internal_command_name} completed"
      return 0
    else
      _afdata_internal_child_status=$?
      if [ -n "$_afdata_internal_output" ]; then
        printf '%s\n' "$_afdata_internal_output" >&2
      fi
      afdata_error child_process_failed \
        "${_afdata_internal_command_name} failed with exit code ${_afdata_internal_child_status}" \
        "inspect the child output above" || :
      return "$_afdata_internal_child_status"
    fi
  fi

  if "$@"; then
    afdata_log info "${_afdata_internal_command_name} completed"
    return 0
  else
    _afdata_internal_child_status=$?
    afdata_error child_process_failed \
      "${_afdata_internal_command_name} failed with exit code ${_afdata_internal_child_status}" \
      "inspect the child output above" || :
    return "$_afdata_internal_child_status"
  fi
}

afdata_args_begin() {
  if [ "$#" -ne 1 ]; then
    _afdata_function_error \
      "afdata_args_begin requires a usage line" \
      "usage: afdata_args_begin <USAGE>"
    return 2
  fi

  AFDATA_ARGS_USAGE="$1"
  AFDATA_OUTPUT="${AFDATA_OUTPUT:-json}"
  AFDATA_OUTPUT_TO="${AFDATA_OUTPUT_TO:-split}"
  _AFDATA_ARGS_OPTION_VARS=()
  _AFDATA_ARGS_OPTION_FLAGS=()
  _AFDATA_ARGS_OPTION_VALUE_NAMES=()
  _AFDATA_ARGS_OPTION_DESCRIPTIONS=()
  _AFDATA_ARGS_OPTION_DEFAULTS=()
  _AFDATA_ARGS_FLAG_VARS=()
  _AFDATA_ARGS_FLAG_FLAGS=()
  _AFDATA_ARGS_FLAG_DESCRIPTIONS=()
  _AFDATA_ARGS_POSITIONAL_VARS=()
  _AFDATA_ARGS_POSITIONAL_NAMES=()
  _AFDATA_ARGS_POSITIONAL_DESCRIPTIONS=()
  _AFDATA_ARGS_POSITIONAL_MODES=()
  _AFDATA_ARGS_REST_NAME=""
  _AFDATA_ARGS_REST_DESCRIPTION=""
  AFDATA_ARGS_REST=()
}

_afdata_args_valid_var() {
  [[ "$1" =~ ^[a-zA-Z_][a-zA-Z0-9_]*$ ]] \
    && [[ "$1" != _afdata_* ]] \
    && [[ "$1" != _AFDATA_* ]] \
    && [[ "$1" != AFDATA_* ]]
}

_afdata_args_valid_flag() {
  [[ "$1" =~ ^--[a-z][a-z0-9]*(-[a-z0-9]+)*$ ]]
}

_afdata_args_flag_in_use() {
  local candidate="$1"
  local index
  case "$candidate" in
    --help|--output|--output-to) return 0 ;;
  esac
  for ((index = 0; index < ${#_AFDATA_ARGS_OPTION_FLAGS[@]}; index++)); do
    [ "${_AFDATA_ARGS_OPTION_FLAGS[index]}" = "$candidate" ] && return 0
  done
  for ((index = 0; index < ${#_AFDATA_ARGS_FLAG_FLAGS[@]}; index++)); do
    [ "${_AFDATA_ARGS_FLAG_FLAGS[index]}" = "$candidate" ] && return 0
  done
  return 1
}

_afdata_args_var_in_use() {
  local candidate="$1"
  local index
  for ((index = 0; index < ${#_AFDATA_ARGS_OPTION_VARS[@]}; index++)); do
    [ "${_AFDATA_ARGS_OPTION_VARS[index]}" = "$candidate" ] && return 0
  done
  for ((index = 0; index < ${#_AFDATA_ARGS_FLAG_VARS[@]}; index++)); do
    [ "${_AFDATA_ARGS_FLAG_VARS[index]}" = "$candidate" ] && return 0
  done
  for ((index = 0; index < ${#_AFDATA_ARGS_POSITIONAL_VARS[@]}; index++)); do
    [ "${_AFDATA_ARGS_POSITIONAL_VARS[index]}" = "$candidate" ] && return 0
  done
  return 1
}

afdata_args_option() {
  if [ "$#" -lt 4 ] || [ "$#" -gt 5 ]; then
    _afdata_function_error \
      "afdata_args_option requires VARIABLE, FLAG, VALUE_NAME, DESCRIPTION, and an optional DEFAULT" \
      "usage: afdata_args_option <VARIABLE> <--long-flag> <VALUE_NAME> <DESCRIPTION> [DEFAULT]"
    return 2
  fi
  if ! _afdata_args_valid_var "$1"; then
    _afdata_function_error "invalid or reserved Bash variable name '$1'" \
      "use snake_case without the _afdata_, _AFDATA_, or AFDATA_ prefix"
    return 2
  fi
  if ! _afdata_args_valid_flag "$2"; then
    _afdata_function_error "invalid AFDATA flag '$2'" "use a long kebab-case flag, for example --config-path"
    return 2
  fi
  if _afdata_args_var_in_use "$1" || _afdata_args_flag_in_use "$2"; then
    _afdata_function_error "duplicate argument declaration '$1' / '$2'" "use a unique variable and flag"
    return 2
  fi

  local _afdata_internal_index="${#_AFDATA_ARGS_OPTION_VARS[@]}"
  _AFDATA_ARGS_OPTION_VARS[_afdata_internal_index]="$1"
  _AFDATA_ARGS_OPTION_FLAGS[_afdata_internal_index]="$2"
  _AFDATA_ARGS_OPTION_VALUE_NAMES[_afdata_internal_index]="$3"
  _AFDATA_ARGS_OPTION_DESCRIPTIONS[_afdata_internal_index]="$4"
  _AFDATA_ARGS_OPTION_DEFAULTS[_afdata_internal_index]="${5-}"
  printf -v "$1" '%s' "${5-}"
}

afdata_args_flag() {
  if [ "$#" -ne 3 ]; then
    _afdata_function_error \
      "afdata_args_flag requires VARIABLE, FLAG, and DESCRIPTION" \
      "usage: afdata_args_flag <VARIABLE> <--long-flag> <DESCRIPTION>"
    return 2
  fi
  if ! _afdata_args_valid_var "$1"; then
    _afdata_function_error "invalid or reserved Bash variable name '$1'" \
      "use snake_case without the _afdata_, _AFDATA_, or AFDATA_ prefix"
    return 2
  fi
  if ! _afdata_args_valid_flag "$2"; then
    _afdata_function_error "invalid AFDATA flag '$2'" "use a long kebab-case flag, for example --dry-run"
    return 2
  fi
  if _afdata_args_var_in_use "$1" || _afdata_args_flag_in_use "$2"; then
    _afdata_function_error "duplicate argument declaration '$1' / '$2'" "use a unique variable and flag"
    return 2
  fi

  local _afdata_internal_index="${#_AFDATA_ARGS_FLAG_VARS[@]}"
  _AFDATA_ARGS_FLAG_VARS[_afdata_internal_index]="$1"
  _AFDATA_ARGS_FLAG_FLAGS[_afdata_internal_index]="$2"
  _AFDATA_ARGS_FLAG_DESCRIPTIONS[_afdata_internal_index]="$3"
  printf -v "$1" '%s' false
}

afdata_args_positional() {
  if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
    _afdata_function_error \
      "afdata_args_positional requires VARIABLE, NAME, DESCRIPTION, and optional|required" \
      "usage: afdata_args_positional <VARIABLE> <NAME> <DESCRIPTION> [required|optional]"
    return 2
  fi
  if ! _afdata_args_valid_var "$1"; then
    _afdata_function_error "invalid or reserved Bash variable name '$1'" \
      "use snake_case without the _afdata_, _AFDATA_, or AFDATA_ prefix"
    return 2
  fi
  if _afdata_args_var_in_use "$1"; then
    _afdata_function_error "duplicate argument variable '$1'" "use a unique Bash variable"
    return 2
  fi

  local _afdata_internal_mode="${4:-required}"
  if [ "$_afdata_internal_mode" != required ] && [ "$_afdata_internal_mode" != optional ]; then
    _afdata_function_error "invalid positional mode '$_afdata_internal_mode'" "valid modes: required, optional"
    return 2
  fi
  local _afdata_internal_index
  for ((_afdata_internal_index = 0; _afdata_internal_index < ${#_AFDATA_ARGS_POSITIONAL_MODES[@]}; _afdata_internal_index++)); do
    if [ "${_AFDATA_ARGS_POSITIONAL_MODES[_afdata_internal_index]}" = optional ] \
      && [ "$_afdata_internal_mode" = required ]; then
      _afdata_function_error \
        "required positional '$2' cannot follow an optional positional" \
        "declare required positional arguments first"
      return 2
    fi
  done

  _afdata_internal_index="${#_AFDATA_ARGS_POSITIONAL_VARS[@]}"
  _AFDATA_ARGS_POSITIONAL_VARS[_afdata_internal_index]="$1"
  _AFDATA_ARGS_POSITIONAL_NAMES[_afdata_internal_index]="$2"
  _AFDATA_ARGS_POSITIONAL_DESCRIPTIONS[_afdata_internal_index]="$3"
  _AFDATA_ARGS_POSITIONAL_MODES[_afdata_internal_index]="$_afdata_internal_mode"
  printf -v "$1" '%s' ""
}

afdata_args_rest() {
  if [ "$#" -ne 2 ]; then
    _afdata_function_error \
      "afdata_args_rest requires NAME and DESCRIPTION" \
      "usage: afdata_args_rest <NAME> <DESCRIPTION>"
    return 2
  fi
  if [ -n "$_AFDATA_ARGS_REST_NAME" ]; then
    _afdata_function_error "duplicate rest argument declaration '$1'" "declare at most one rest argument"
    return 2
  fi
  _AFDATA_ARGS_REST_NAME="$1"
  _AFDATA_ARGS_REST_DESCRIPTION="$2"
  AFDATA_ARGS_REST=()
}

afdata_args_help() {
  local index
  local label
  printf 'Usage: %s\n' "${AFDATA_ARGS_USAGE:-${0##*/}}"

  if [ "${#_AFDATA_ARGS_POSITIONAL_VARS[@]}" -gt 0 ]; then
    printf '\nArguments:\n'
    for ((index = 0; index < ${#_AFDATA_ARGS_POSITIONAL_VARS[@]}; index++)); do
      label="${_AFDATA_ARGS_POSITIONAL_NAMES[index]}"
      if [ "${_AFDATA_ARGS_POSITIONAL_MODES[index]}" = optional ]; then
        label="[${label}]"
      fi
      printf '  %-24s %s\n' "$label" "${_AFDATA_ARGS_POSITIONAL_DESCRIPTIONS[index]}"
    done
  fi
  if [ -n "$_AFDATA_ARGS_REST_NAME" ]; then
    [ "${#_AFDATA_ARGS_POSITIONAL_VARS[@]}" -gt 0 ] || printf '\nArguments:\n'
    printf '  %-24s %s\n' "[${_AFDATA_ARGS_REST_NAME} ...]" "$_AFDATA_ARGS_REST_DESCRIPTION"
  fi

  printf '\nOptions:\n'
  for ((index = 0; index < ${#_AFDATA_ARGS_OPTION_VARS[@]}; index++)); do
    label="${_AFDATA_ARGS_OPTION_FLAGS[index]} ${_AFDATA_ARGS_OPTION_VALUE_NAMES[index]}"
    printf '  %-24s %s\n' "$label" "${_AFDATA_ARGS_OPTION_DESCRIPTIONS[index]}"
  done
  for ((index = 0; index < ${#_AFDATA_ARGS_FLAG_VARS[@]}; index++)); do
    printf '  %-24s %s\n' \
      "${_AFDATA_ARGS_FLAG_FLAGS[index]}" \
      "${_AFDATA_ARGS_FLAG_DESCRIPTIONS[index]}"
  done
  printf '  %-24s %s\n' '--output FORMAT' 'Output format: json, yaml, or plain'
  printf '  %-24s %s\n' '--output-to DEST' 'Event destination: split, stdout, or stderr'
  printf '  %-24s %s\n' '--help' 'Print help'
}

_afdata_args_abort() {
  local message="$1"
  local hint="try: ${0##*/} --help"
  if ! afdata_error cli_error "$message" "$hint"; then
    :
  fi
  exit 2
}

_afdata_args_set_output() {
  case "$1" in
    json|yaml|plain) _afdata_internal_next_output="$1" ;;
    *) _afdata_args_abort "invalid --output value; valid values: json, yaml, plain" ;;
  esac
}

_afdata_args_set_output_to() {
  case "$1" in
    split|stdout|stderr) _afdata_internal_next_output_to="$1" ;;
    *) _afdata_args_abort "invalid --output-to value; valid values: split, stdout, stderr" ;;
  esac
}

# Parse arguments for an executable script. Like conventional argument parsers,
# this exits the script with 0 for --help and 2 for malformed arguments.
afdata_args_parse() {
  # Every parser-local name uses a reserved prefix. Bash has dynamic scope, so
  # a plain local such as `mode` would otherwise intercept assignment to an
  # application variable with that name on Bash 3.2 (which lacks namerefs).
  local _afdata_internal_positional_only=false
  local _afdata_internal_positional_index=0
  local _afdata_internal_arg
  local _afdata_internal_flag
  local _afdata_internal_value
  local _afdata_internal_matched
  local _afdata_internal_index
  local _afdata_internal_variable
  local _afdata_internal_scan
  local _afdata_internal_saw_help=false
  local _afdata_internal_saw_output=false
  # Output selectors become trusted only after the entire argv is valid.
  # Until then usage errors keep the caller's pre-parse routing instead of
  # letting a later-invalid argv redirect or reformat its own diagnostic.
  local _afdata_internal_next_output="$AFDATA_OUTPUT"
  local _afdata_internal_next_output_to="$AFDATA_OUTPUT_TO"

  # Help is raw human text, while output selection promises structured data.
  # Reject the contradictory request before either branch can win by argv
  # order. Tokens after `--` are application positionals and do not count.
  for _afdata_internal_scan in "$@"; do
    case "$_afdata_internal_scan" in
      --) break ;;
      --help) _afdata_internal_saw_help=true ;;
      --output|--output=*|--output-to|--output-to=*)
        _afdata_internal_saw_output=true
        ;;
    esac
  done
  if [ "$_afdata_internal_saw_help" = true ] \
    && [ "$_afdata_internal_saw_output" = true ]; then
    _afdata_args_abort "help cannot be combined with --output or --output-to"
  fi

  while [ "$#" -gt 0 ]; do
    _afdata_internal_arg="$1"
    shift

    if [ "$_afdata_internal_positional_only" = false ]; then
      case "$_afdata_internal_arg" in
        --help)
          afdata_args_help
          exit 0
          ;;
        --)
          _afdata_internal_positional_only=true
          continue
          ;;
        --output)
          [ "$#" -gt 0 ] || _afdata_args_abort "--output requires FORMAT"
          _afdata_args_set_output "$1"
          shift
          continue
          ;;
        --output=*)
          _afdata_args_set_output "${_afdata_internal_arg#*=}"
          continue
          ;;
        --output-to)
          [ "$#" -gt 0 ] || _afdata_args_abort "--output-to requires DEST"
          _afdata_args_set_output_to "$1"
          shift
          continue
          ;;
        --output-to=*)
          _afdata_args_set_output_to "${_afdata_internal_arg#*=}"
          continue
          ;;
      esac

      if [[ "$_afdata_internal_arg" == --*=* ]]; then
        _afdata_internal_flag="${_afdata_internal_arg%%=*}"
        _afdata_internal_value="${_afdata_internal_arg#*=}"
        _afdata_internal_matched=false
        for ((_afdata_internal_index = 0; _afdata_internal_index < ${#_AFDATA_ARGS_OPTION_FLAGS[@]}; _afdata_internal_index++)); do
          if [ "${_AFDATA_ARGS_OPTION_FLAGS[_afdata_internal_index]}" = "$_afdata_internal_flag" ]; then
            _afdata_internal_variable="${_AFDATA_ARGS_OPTION_VARS[_afdata_internal_index]}"
            printf -v "$_afdata_internal_variable" '%s' "$_afdata_internal_value"
            _afdata_internal_matched=true
            break
          fi
        done
        if [ "$_afdata_internal_matched" = false ]; then
          for ((_afdata_internal_index = 0; _afdata_internal_index < ${#_AFDATA_ARGS_FLAG_FLAGS[@]}; _afdata_internal_index++)); do
            if [ "${_AFDATA_ARGS_FLAG_FLAGS[_afdata_internal_index]}" = "$_afdata_internal_flag" ]; then
              _afdata_args_abort "$_afdata_internal_flag does not take a value"
            fi
          done
          _afdata_args_abort "unknown option '$_afdata_internal_flag'"
        fi
        continue
      fi

      if [[ "$_afdata_internal_arg" == --* ]]; then
        _afdata_internal_matched=false
        for ((_afdata_internal_index = 0; _afdata_internal_index < ${#_AFDATA_ARGS_FLAG_FLAGS[@]}; _afdata_internal_index++)); do
          if [ "${_AFDATA_ARGS_FLAG_FLAGS[_afdata_internal_index]}" = "$_afdata_internal_arg" ]; then
            _afdata_internal_variable="${_AFDATA_ARGS_FLAG_VARS[_afdata_internal_index]}"
            printf -v "$_afdata_internal_variable" '%s' true
            _afdata_internal_matched=true
            break
          fi
        done
        if [ "$_afdata_internal_matched" = true ]; then
          continue
        fi
        for ((_afdata_internal_index = 0; _afdata_internal_index < ${#_AFDATA_ARGS_OPTION_FLAGS[@]}; _afdata_internal_index++)); do
          if [ "${_AFDATA_ARGS_OPTION_FLAGS[_afdata_internal_index]}" = "$_afdata_internal_arg" ]; then
            [ "$#" -gt 0 ] || _afdata_args_abort \
              "$_afdata_internal_arg requires ${_AFDATA_ARGS_OPTION_VALUE_NAMES[_afdata_internal_index]}"
            _afdata_internal_variable="${_AFDATA_ARGS_OPTION_VARS[_afdata_internal_index]}"
            printf -v "$_afdata_internal_variable" '%s' "$1"
            shift
            _afdata_internal_matched=true
            break
          fi
        done
        [ "$_afdata_internal_matched" = true ] \
          || _afdata_args_abort "unknown option '$_afdata_internal_arg'"
        continue
      fi

      if [[ "$_afdata_internal_arg" == -* ]]; then
        _afdata_args_abort "unknown short option; use long kebab-case flags"
      fi
    fi

    if [ "$_afdata_internal_positional_index" -ge "${#_AFDATA_ARGS_POSITIONAL_VARS[@]}" ]; then
      if [ -n "$_AFDATA_ARGS_REST_NAME" ]; then
        AFDATA_ARGS_REST[${#AFDATA_ARGS_REST[@]}]="$_afdata_internal_arg"
        continue
      fi
      _afdata_args_abort "unexpected positional argument"
    fi
    _afdata_internal_variable="${_AFDATA_ARGS_POSITIONAL_VARS[_afdata_internal_positional_index]}"
    printf -v "$_afdata_internal_variable" '%s' "$_afdata_internal_arg"
    _afdata_internal_positional_index=$((_afdata_internal_positional_index + 1))
  done

  for ((_afdata_internal_index = _afdata_internal_positional_index; _afdata_internal_index < ${#_AFDATA_ARGS_POSITIONAL_VARS[@]}; _afdata_internal_index++)); do
    if [ "${_AFDATA_ARGS_POSITIONAL_MODES[_afdata_internal_index]}" = required ]; then
      _afdata_args_abort \
        "missing required argument ${_AFDATA_ARGS_POSITIONAL_NAMES[_afdata_internal_index]}"
    fi
  done

  AFDATA_OUTPUT="$_afdata_internal_next_output"
  AFDATA_OUTPUT_TO="$_afdata_internal_next_output_to"

  # AFDATA-aware child commands inherit the caller's selected event routing.
  export AFDATA_OUTPUT AFDATA_OUTPUT_TO
}
