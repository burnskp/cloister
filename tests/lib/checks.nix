{ lib, pkgs }:
let
  render = value: builtins.toJSON value;
in
{
  expectTrue =
    label: condition: if condition then true else throw "${label}: expected condition to be true";

  expectFalse =
    label: condition: if condition then throw "${label}: expected condition to be false" else true;

  expectEq =
    label: expected: actual:
    if expected == actual then
      true
    else
      throw "${label}: expected ${render expected}, got ${render actual}";

  expectContains =
    label: needle: haystack:
    if lib.hasInfix needle haystack then
      true
    else
      throw "${label}: expected to find ${render needle} in ${render haystack}";

  expectNotContains =
    label: needle: haystack:
    if lib.hasInfix needle haystack then
      throw "${label}: did not expect to find ${render needle} in ${render haystack}"
    else
      true;

  expectFailure =
    label: value:
    let
      result = builtins.tryEval (builtins.deepSeq value true);
    in
    if result.success then throw "${label}: expected evaluation failure" else true;

  expectAssertionMessage =
    label: assertions: needle:
    let
      failedMessages = map (assertion: assertion.message) (
        builtins.filter (assertion: !assertion.assertion) assertions
      );
    in
    if builtins.any (message: lib.hasInfix needle message) failedMessages then
      true
    else
      throw "${label}: expected failing assertion containing ${render needle}, got ${render failedMessages}";

  mkCheck =
    name: assertions:
    builtins.deepSeq assertions (
      pkgs.runCommand name { } ''
        touch "$out"
      ''
    );
}
