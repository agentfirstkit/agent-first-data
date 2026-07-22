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
AFDATA_BASH_API_VERSION=1

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

_afdata_emit() {
  afdata_cli \
    --output "${AFDATA_OUTPUT:-json}" \
    --output-to "${AFDATA_OUTPUT_TO:-split}" \
    emit "$@"
}

afdata_log() {
  if [ "$#" -ne 2 ]; then
    _afdata_function_error \
      "afdata_log requires LEVEL and MESSAGE" \
      "usage: afdata_log <debug|info|warn|error> <MESSAGE>"
    return 2
  fi
  _afdata_emit log "$1" "$2"
}

afdata_result() {
  if [ "$#" -ne 1 ]; then
    _afdata_function_error \
      "afdata_result requires MESSAGE" \
      "usage: afdata_result <MESSAGE>"
    return 2
  fi
  # A child invoked through afdata_call participates in its parent's finite
  # event stream. The outermost script owns the unique terminal result, so a
  # successful child completion is diagnostic rather than terminal.
  if [ "${_AFDATA_BASH_CHILD:-0}" = "1" ]; then
    afdata_log info "$1"
    return $?
  fi
  _afdata_emit result "$1"
}

afdata_error() {
  if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    _afdata_function_error \
      "afdata_error requires CODE, MESSAGE, and an optional HINT" \
      "usage: afdata_error <CODE> <MESSAGE> [HINT]"
    return 2
  fi
  if [ "$#" -eq 3 ]; then
    _afdata_emit error "$1" "$2" --hint "$3"
  else
    _afdata_emit error "$1" "$2"
  fi
}

_afdata_function_error() {
  local message="$1"
  local hint="$2"
  if ! _afdata_emit error cli_error "$message" --hint "$hint"; then
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
  printf '  %-24s %s\n' '-h, --help' 'Print help'
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
    json|yaml|plain) AFDATA_OUTPUT="$1" ;;
    *) _afdata_args_abort "invalid --output '$1'; valid values: json, yaml, plain" ;;
  esac
}

_afdata_args_set_output_to() {
  case "$1" in
    split|stdout|stderr) AFDATA_OUTPUT_TO="$1" ;;
    *) _afdata_args_abort "invalid --output-to '$1'; valid values: split, stdout, stderr" ;;
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

  while [ "$#" -gt 0 ]; do
    _afdata_internal_arg="$1"
    shift

    if [ "$_afdata_internal_positional_only" = false ]; then
      case "$_afdata_internal_arg" in
        -h|--help)
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
        _afdata_args_abort "unknown short option '$_afdata_internal_arg'; use long kebab-case flags"
      fi
    fi

    if [ "$_afdata_internal_positional_index" -ge "${#_AFDATA_ARGS_POSITIONAL_VARS[@]}" ]; then
      if [ -n "$_AFDATA_ARGS_REST_NAME" ]; then
        AFDATA_ARGS_REST[${#AFDATA_ARGS_REST[@]}]="$_afdata_internal_arg"
        continue
      fi
      _afdata_args_abort "unexpected positional argument '$_afdata_internal_arg'"
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

  # AFDATA-aware child commands inherit the caller's selected event routing.
  export AFDATA_OUTPUT AFDATA_OUTPUT_TO
}
