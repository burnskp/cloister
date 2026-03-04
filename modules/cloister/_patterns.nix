# Shared regex patterns for name validation.
# Used in _registry.nix (assertion checks) and _options.nix (documentation).
{
  sandboxName = "^[A-Za-z0-9_-]+$";
  safeAlias = "^[A-Za-z_][A-Za-z0-9._+-]*$";
  safeFunction = "^[A-Za-z_][A-Za-z0-9_]*$";
  safeCommand = "^[A-Za-z_][A-Za-z0-9._+-]*$";
}
