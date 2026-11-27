# Game source helpers

Source-only helpers that the mod API generator and CI's ignored-test inventory share. They read source with `syn`
and never build it. No game feature names, platform dependencies or unsafe code.

- `native_hook_files`: the hand-written files of `game/native_hooks/`, in path order.
- `module_files`: a folder's Rust file stems, sorted and checked to be identifiers.
- `exports`: the vocabulary's export fragments.
- The `sunrise-game-discovery` binary: the ignored-test inventory (`tools/ci/ignored_inventory.py`). It follows each
  crate's `mod` items, `#[path]` included, as the compiler does. With `--references --boundary FILE::NAME
  [--literal TEXT]... [--fragment TEXT]...` it writes instead which functions, constants and macros of each target
  reach a seed (`references.rs`): the boundary function, a function that spells a string literal, a string that
  contains a fragment, then one `target_files` record (the target's source files) for each target that spells a
  seed; paths are repository-relative with `/` separators. It approximates names within one crate (modules, `use`
  with renames, groups and re-exports, `super::*` globs, `Self::`, `<Type as Trait>::function`, `type` aliases of a
  path, `self.method()`, `Type { ... }.method()`, `macro_rules!`). A `use` inside a function binds
  for that function only. Some unresolved paths are reported when their names match known fixture readers, and only readers visible from the module that
  spells them: a private item is visible in its module and below, `pub(super)` in the parent's subtree,
  `pub(crate)`, `pub` and `pub(in ..)` in the crate, widened by a `use` that re-exports it, and a method is as
  visible as its type. Only functions with a `self` receiver are reached by a method call (`Type::new` is
  never `.new()`), and a concrete type that lacks a function is refused only when a visible trait's default body
  of that name reaches a seed. Unknown-receiver method lists and unresolved-path comparisons exclude test-only
  candidates for production callers. Test context comes from an integration test target, `#[test]`, or a
  `#[cfg(test)]` gate (including a `test` requirement within `all(...)`) on an item, its enclosing impl or
  type, or a containing module. Macro token scans skip struct field labels but still scan their values.
  The scanner does not follow dependencies or model lexical shadowing, macro expansion, general receiver types,
  or other `cfg` predicates. `tools/ci/check_ignored_test_rows.py` applies the policy.

The game's modules are ordinary `mod` items: `game.rs` names each module of `game/<area>/` with a `#[path]`, so an area
folder groups files without becoming a module.
