# AGENTS

Guidance for automated assistants working in this repository.
`Prefer` and `consider` mark defaults that allow task-specific judgment; other directives are requirements
within their stated scope.

Shroom is an SSH-first workspace manager with a thin, cross-platform Rust core over microsandbox.
The [Shroom core reference](docs/references/shroom-core.md#ssh-workspace-core) defines its scope, SSH access contract,
and the responsibilities delegated to the SDK.

## Language and Terminology

Use English for identifiers and repository prose, including comments and documentation.
Communicate in the user's preferred language, following the current conversation when none is stated.
Keep established technical terms in English when that preserves precision and searchability,
unless the user requests localization.

## Working Principles

- When recurring cases support a shared rule, state it and suggest the abstraction or convention that follows.
  Distinguish evidence from inference, and favor connections that simplify future decisions.
  Do not invent abstractions merely to claim novelty.
- When replacing a design, update its callers and remove the superseded path in the same change.
  Retain compatibility layers only when the user explicitly requests a compatibility boundary;
  state its scope and intended removal condition.
- When changing validation or behavior with rejection cases, pair valid inputs with rejected counterparts.
  Assert the intended error and relevant failure invariants.
  Retain bug reproducers as regression tests.

## Rust Conventions

Register every Rust dependency in the root `Cargo.toml` under `[workspace.dependencies]`;
crates refer to it with `dependency = { workspace = true }`.

Represent semantic states, internal messages, and errors with domain types.
Parse external text into structured data early; use strings when the data itself is textual.
Introduce types as needed. Before using a string map, distinguish text storage or interning
from structured data better modeled with structs or traits.

Prefer iterator transformations such as `map`, fallible `collect`, `unzip`, and `fold` over mutable accumulators.
Keep mutation for essential sequential state or when a functional rewrite would materially harm clarity or performance.

Group functions on structs as methods or associated functions.
Use free functions only when required by the language, a macro, or an external interface.
Use `self` when consuming a value, `&self` or `&mut self` when borrowing it, and associated functions
when the struct serves as a namespace.

Prefer direct struct construction unless a constructor performs additional work or establishes a visibility boundary.
Do not add a lone `new` method that merely repackages a struct literal.
Name general constructors `new` and constructors indicating how a value is created `with_*`.

Give builders an associated entry point `fn new(required, ...)`.
Choose receivers according to whether `build` or `finish` moves owned fields:

- For consuming builders, use `fn build(self) -> T` and setters `fn with_*(mut self, ...) -> Self`.
- For builders that can borrow, prefer `fn build(&self) -> T` and setters `fn set_*(&mut self, ...) -> &mut Self`.

Use `with_*` and `set_*` consistently for optional configuration.
Prefer `derive` and `derive_more` when generated behavior exactly matches the intended semantics;
write manual implementations only when additional invariants require them.

## Documentation Structure and Design Records

Organize documentation under `docs/`, with `docs/README.md` as its index:

- `docs/references/`: the sole authoritative home for all user-approved canonical design decisions,
  including workspace semantics, application architecture, and invariants.
- `docs/proposals/`: concrete design documents for ongoing features not yet implemented.
- `docs/ideas/`: higher-level academic discussions, research questions, and conceptual exploration.
- `docs/evaluations/`: dated experimental reports and their reproducible evidence, with one directory per study.
- `docs/todos/`: urgent codebase drift from the references.
  Link the governing rule, describe the discrepancy, and address it as soon as possible.

Adopting, changing, or promoting a canonical design requires user permission; existing authorization suffices.
Once a feature is implemented, move its approved, settled design into the references.
Keep only ongoing, unimplemented design work in proposals.

Maintain **one home per canonical rule**, in the reference section that owns its semantic boundary.
Outside references, prefer concise pointers over repeated design explanations.
Keep dependent examples and links consistent with the canonical rule.
When moving or merging content, remove superseded sections or files and update incoming links in the same change.

Scratch records in `docs/logs/` are temporary; fold durable content into its proper home, then delete the log.

## Documentation Style

Establish motivation and context before introducing machinery; explain which question each mechanism answers.
Introduce concepts before relying on them, and let information density rise gradually.
Connect paragraphs with reasoning that explains the next step, especially before increasing technical detail.
Use technical terminology where precise, explaining specialized terms at first use with source-level intuition
or a concrete example.

Give each paragraph one overarching point.
Develop it through motivation, mechanism, evidence, or consequence; split the paragraph when its question changes.
Use lists or tables for independent inventories.
Let paragraph and sentence lengths follow the argument, avoiding formulaic rhythms and repeated contrastive phrasing.
Use parallel construction when it clarifies a comparison, enumeration, or invariant.

Preserve the short explanations that connect technical facts when compressing prose.
Use direct subjects and verbs, and name concrete operations.
Connectives should express the actual logical relationship between statements.

Prefer positive explanations. Retain negation that clarifies a live alternative, formal exclusion,
or implementation limit.

During prose-only revisions, preserve technical scope, equations, examples, and established terminology.
After revising, read the document from beginning to end to check its progression and concept order.

Wrap prose around 90–120 characters, breaking after sentence or clause punctuation, or before connectives.
Treat commas as ordinary clause boundaries. Do not enforce one sentence per line.
Keep complete sentences together when they fit; allow short final lines instead of splitting phrases to balance widths.
Preserve code blocks and other structural Markdown.

Capitalize titles in `docs/references/` with Chicago title case: capitalize most words,
including both parts of hyphenated compounds; keep short function words (a, an, the, and, or, of, in, to,
as, with) lowercase unless first; preserve code terms' exact spelling regardless of position.

## Commit Messages

Use `prefix: lowercase description`, on one line with no trailing period.
Describe what changed. For `feat`, name the capability directly; use a leading `add` only when it improves clarity.

| Prefix | When to use |
|--------|-------------|
| `feat` | A new user-visible capability. |
| `incr` | Incremental progress: fixes, polish, tuning, or small feature additions. |
| `sisy` | Mechanical changes or internal restructuring with no behavior change. |
| `vibe` | Exploratory or prototype work that may be revised or replaced. |
| `repo` | Repository housekeeping, dependencies, migrations, or file reorganization. |
| `docs` | Documentation-only changes, including Rust docs and comments. |
| `test` | Test changes without production code changes. |
