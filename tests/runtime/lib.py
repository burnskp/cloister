import json


def _format_failure(label, command, expected, actual):
    parts = []
    if label:
        parts.append(label)
    parts.append(f"command: {command}")
    parts.append(f"expected: {expected}")
    parts.append("actual:")
    parts.append(actual.rstrip() if actual.rstrip() else "<empty output>")
    return "\n".join(parts)


def assert_contains(machine, command, needle, label=None):
    actual = machine.succeed(command)
    if needle not in actual:
        raise AssertionError(
            _format_failure(label, command, f"output containing {needle!r}", actual)
        )
    return actual


def assert_failure(machine, command, label=None):
    wrapped = (
        "set +e; output=$( ("
        + command
        + ") 2>&1 ); status=$?; printf '%s' \"$output\"; test $status -ne 0"
    )
    actual = machine.succeed(wrapped)
    if not actual and label:
        return actual
    return actual


def assert_failure_contains(machine, command, needle, label=None):
    actual = assert_failure(machine, command, label)
    if needle not in actual:
        raise AssertionError(
            _format_failure(
                label,
                command,
                f"failing output containing {needle!r}",
                actual,
            )
        )
    return actual


def assert_not_contains(machine, command, needle, label=None):
    actual = machine.succeed(command)
    if needle in actual:
        raise AssertionError(
            _format_failure(
                label,
                command,
                f"output not containing {needle!r}",
                actual,
            )
        )
    return actual


def assert_eq(machine, command, expected, label=None):
    actual = machine.succeed(command)
    normalized = actual.rstrip("\n")
    if normalized != expected:
        raise AssertionError(
            _format_failure(label, command, f"exact output {expected!r}", actual)
        )
    return actual


def assert_json_eq(machine, command, expected, label=None):
    actual = machine.succeed(command)
    parsed = json.loads(actual)
    if parsed != expected:
        raise AssertionError(
            _format_failure(
                label,
                command,
                f"JSON equal to {json.dumps(expected, sort_keys=True)}",
                json.dumps(parsed, indent=2, sort_keys=True),
            )
        )
    return parsed
