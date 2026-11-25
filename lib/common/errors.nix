# User-facing errors share a prefix and, for a request no pin satisfies, a
# shape. Keeping both here means every toolchain tells the user the same thing
# and no call site can forget the `crucible:` prefix or the next step that
# makes the error actionable.
_:
let
  # Throw a message that is prefixed with the tool name users know. Callers
  # write only the body, so the prefix cannot drift between languages.
  fail = message: throw "crucible: ${message}";

  # The shape shared by languages that resolve a version against a set of
  # pinned releases. `releases` is the ready-joined list of pins and `next`
  # is the language-specific way to obtain an unpinned one; both keep their
  # trailing text exactly so existing messages stay byte-for-byte identical.
  notPinned =
    {
      name,
      spec,
      releases,
      next,
    }:
    fail ("${name} \"${spec}\" matches no pinned release (${releases}).\n" + next);
in
{
  inherit fail notPinned;
}
