# Just enough of npm's semver ranges to pick a pinned Node.js release for
# `engines.node` or `devEngines.runtime.version`, without asking the network
# what exists. Pre-release tags are ignored: only stable releases are pinned.
{ lib }:
let
  inherit (builtins)
    elemAt
    filter
    length
    match
    ;
  inherit (lib)
    all
    any
    splitString
    toInt
    trim
    ;

  wildcard = x: x == null || x == "x" || x == "X" || x == "*";

  # "1.2.x" -> [ 1 2 null ]; a missing or wildcard part is null.
  partial =
    s:
    let
      part = "([0-9]+|[xX*])";
      m = match "v?${part}?(\\.${part})?(\\.${part})?(-[0-9A-Za-z.-]+)?(\\+[0-9A-Za-z.-]+)?" s;
      num = x: if wildcard x then null else toInt x;
    in
    if m == null then
      throw "crucible: \"${s}\" is not a version npm understands"
    else
      [
        (num (elemAt m 0))
        (num (elemAt m 2))
        (num (elemAt m 4))
      ];

  # -1, 0 or 1, comparing [ major minor patch ] lists.
  compare =
    a: b:
    if a == [ ] then
      0
    else if lib.head a < lib.head b then
      -1
    else if lib.head a > lib.head b then
      1
    else
      compare (lib.tail a) (lib.tail b);

  ge = v: {
    inherit v;
    ok = c: c >= 0;
  };
  gt = v: {
    inherit v;
    ok = c: c > 0;
  };
  le = v: {
    inherit v;
    ok = c: c <= 0;
  };
  lt = v: {
    inherit v;
    ok = c: c < 0;
  };
  never = [
    (lt [
      0
      0
      0
    ])
  ];

  # One comparator ("^1.2", ">=22", "1.x") as bounds that must all hold.
  bounds =
    op: s:
    let
      p = partial s;
      M = elemAt p 0;
      m = elemAt p 1;
      P = elemAt p 2;
      fill = map (x: if x == null then 0 else x) p;
    in
    if op == "" || op == "=" then
      if M == null then
        [ ]
      else if m == null then
        [
          (ge fill)
          (lt [
            (M + 1)
            0
            0
          ])
        ]
      else if P == null then
        [
          (ge fill)
          (lt [
            M
            (m + 1)
            0
          ])
        ]
      else
        [
          (ge fill)
          (le fill)
        ]
    else if op == ">=" then
      [ (ge fill) ]
    else if op == ">" then
      if M == null then
        never
      else if m == null then
        [
          (ge [
            (M + 1)
            0
            0
          ])
        ]
      else if P == null then
        [
          (ge [
            M
            (m + 1)
            0
          ])
        ]
      else
        [ (gt fill) ]
    else if op == "<" then
      if M == null then never else [ (lt fill) ]
    else if op == "<=" then
      if M == null then
        [ ]
      else if m == null then
        [
          (lt [
            (M + 1)
            0
            0
          ])
        ]
      else if P == null then
        [
          (lt [
            M
            (m + 1)
            0
          ])
        ]
      else
        [ (le fill) ]
    else if op == "~" || op == "~>" then
      if M == null then
        [ ]
      else if m == null then
        [
          (ge fill)
          (lt [
            (M + 1)
            0
            0
          ])
        ]
      else
        [
          (ge fill)
          (lt [
            M
            (m + 1)
            0
          ])
        ]
    # "^": everything up to the next change of the leftmost non-zero part.
    else if M == null then
      [ ]
    else if M > 0 || m == null then
      [
        (ge fill)
        (lt [
          (M + 1)
          0
          0
        ])
      ]
    else if m > 0 || P == null then
      [
        (ge fill)
        (lt [
          0
          (m + 1)
          0
        ])
      ]
    else
      [
        (ge fill)
        (lt [
          0
          0
          (P + 1)
        ])
      ];

  operator = "(<=|>=|<|>|=|~>|~|\\^)";

  # One space-separated set ("  >= 22  <27 "), as a flat list of bounds. An
  # operator may stand apart from its version (">= 22").
  conjunction =
    s:
    let
      hyphen = match "(.+) +- +(.+)" s;
      words = filter (w: w != "") (splitString " " s);
      # Glue a lone operator to the word after it.
      glued = lib.foldl' (
        acc: w:
        if acc != [ ] && match operator (lib.last acc) != null then
          lib.init acc ++ [ (lib.last acc + w) ]
        else
          acc ++ [ w ]
      ) [ ] words;
      comparator =
        w:
        let
          m = match "${operator}?(.*)" w;
        in
        bounds (if elemAt m 0 == null then "" else elemAt m 0) (elemAt m 1);
    in
    if hyphen != null then
      bounds ">=" (trim (elemAt hyphen 0)) ++ bounds "<=" (trim (elemAt hyphen 1))
    else
      lib.concatMap comparator glued;

  # The range as alternatives, each a list of bounds.
  parse = range: map (s: conjunction (trim s)) (splitString "||" range);

  satisfies =
    version: range:
    let
      v = map toInt (lib.take 3 (splitString "." (lib.removePrefix "v" version)));
    in
    any (all (b: b.ok (compare v b.v))) (parse range);
in
{
  inherit satisfies;

  # The newest of `versions` in `range`, or null.
  newest =
    range: versions:
    let
      matching = filter (v: satisfies v range) versions;
    in
    if length matching == 0 then
      null
    else
      lib.foldl' (
        a: b: if compare (map toInt (splitString "." a)) (map toInt (splitString "." b)) >= 0 then a else b
      ) (lib.head matching) matching;
}
