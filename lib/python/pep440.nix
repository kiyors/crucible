# PEP 440 version specifiers, enough to pick the newest pinned CPython for a
# project's requires-python offline: comma-separated clauses with ==, !=, ~=,
# >=, <=, >, <, === and the ".*" suffix. A bare version without an operator is
# what uv and .python-version mean by it: "3" is any 3.x, "3.14" any 3.14.x and
# "3.14.2" that release.
#
# Only final releases are pinned, so pre-release and local segments in a
# specifier are rejected rather than half-understood.
{ lib }:
let
  inherit (builtins)
    all
    elemAt
    filter
    genList
    head
    length
    match
    ;

  parts = v: map lib.toInt (lib.splitString "." v);

  # PEP 440 pads the shorter release with zeros: 3.14 == 3.14.0.
  compare =
    a: b:
    let
      n = lib.max (length a) (length b);
      pad = l: l ++ genList (_: 0) (n - length l);
      a' = pad a;
      b' = pad b;
      go =
        i:
        if i == n then
          0
        else if elemAt a' i < elemAt b' i then
          -1
        else if elemAt a' i > elemAt b' i then
          1
        else
          go (i + 1);
    in
    go 0;

  # Whether `v` starts with `prefix`, segment by segment.
  hasPrefix = prefix: v: length v >= length prefix && lib.take (length prefix) v == prefix;

  clause =
    spec: raw:
    let
      m = match "(===|==|!=|~=|>=|<=|>|<)?[[:space:]]*v?([0-9]+(\\.[0-9]+)*)(\\.\\*)?" (lib.trim raw);
      op = if m == null then null else elemAt m 0;
      release = if m == null then null else parts (elemAt m 1);
      wildcard = m != null && elemAt m 3 != null;
    in
    if m == null || (wildcard && !(op == "==" || op == "!=")) then
      throw ''
        crucible: Python version "${spec}" is not a version or PEP 440 specifier crucible
        understands (such as "3.14", ">=3.12,<3.15" or "~=3.13.2"; pre-releases are not pinned).
      ''
    else if op == null then
      hasPrefix release
    else if op == "==" || op == "===" then
      if wildcard then hasPrefix release else v: compare v release == 0
    else if op == "!=" then
      if wildcard then v: !hasPrefix release v else v: compare v release != 0
    else if op == "~=" then
      if length release < 2 then
        throw ''
          crucible: "~=" needs at least two parts ("~=3.12"), and "${spec}" has one.
        ''
      else
        v: compare v release >= 0 && hasPrefix (lib.init release) v
    else if op == ">=" then
      v: compare v release >= 0
    else if op == "<=" then
      v: compare v release <= 0
    else if op == ">" then
      v: compare v release > 0
    else
      v: compare v release < 0;

  # Poetry spells a compatible range "^X.Y", which means the same as the PEP
  # 440 pair ">=X.Y,<X+1". Expand it before parsing so the pin solver needs
  # only one clause grammar. Other specifiers pass through unchanged.
  expandCaret =
    spec:
    let
      m = match "\\^[[:space:]]*([0-9]+(\\.[0-9]+)*)(.*)" (lib.trim spec);
    in
    if m == null then
      [ spec ]
    else
      let
        # builtins.match returns capture groups only, not the whole match.
        lower = elemAt m 0;
        upper = toString (lib.toInt (head (lib.splitString "." lower)) + 1);
        rest = lib.trim (elemAt m 2);
      in
      [
        ">=${lower}"
        "<${upper}"
      ]
      ++ lib.optional (rest != "") rest;

  clauses =
    spec:
    map (clause spec) (
      filter (c: lib.trim c != "") (lib.concatMap expandCaret (lib.splitString "," spec))
    );

  satisfies = version: spec: all (ok: ok (parts version)) (clauses spec);
in
{
  inherit satisfies;

  # Poetry's spelling normalised to plain PEP 440, for callers that need to
  # show or validate the spec they chose.
  poetrySpec = spec: lib.concatStringsSep "," (lib.concatMap expandCaret (lib.splitString "," spec));

  # The newest of `versions` that `spec` allows, or null.
  newest =
    spec: versions:
    let
      ok = filter (v: satisfies v spec) versions;
    in
    if ok == [ ] then null else head (lib.sort (a: b: compare (parts a) (parts b) > 0) ok);
}
