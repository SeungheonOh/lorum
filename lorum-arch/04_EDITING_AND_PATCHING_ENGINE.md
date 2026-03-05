# 04 — Editing and Patching Engine (Replace / Patch / Hashline)

This document describes the complete edit pipeline in the current TypeScript implementation and defines parity-critical behavior for a Rust rewrite.

Primary source surface:

- `packages/coding-agent/src/patch/index.ts`
- `packages/coding-agent/src/patch/hashline.ts`
- `packages/coding-agent/src/patch/parser.ts`
- `packages/coding-agent/src/patch/applicator.ts`
- `packages/coding-agent/src/patch/diff.ts`
- `packages/coding-agent/src/patch/fuzzy.ts`
- `packages/coding-agent/src/patch/normalize.ts`
- `packages/coding-agent/src/patch/types.ts`
- `packages/coding-agent/src/patch/shared.ts`
- `packages/coding-agent/src/lsp/*` (writethrough integration points)

---

## 1) Mode model and runtime mode selection

The `edit` tool is a single logical tool with three operational modes:

1. **replace** (text replacement with fuzzy support)
2. **patch** (diff/hunk application)
3. **hashline** (line-addressed edits with hash integrity)

Mode selection details (`patch/index.ts`):

- `DEFAULT_EDIT_MODE = "hashline"`
- dynamic mode getter consults:
  - env override `PI_EDIT_VARIANT` (validated by `normalizeEditMode`)
  - model-specific settings (`settings.getEditVariantForModel(...)`)
  - global `edit.mode`

This means edit semantics can shift by active model; in Rust, mode must remain runtime-resolved, not fixed at startup.

---

## 2) Input schemas by mode

### 2.1 Replace mode schema

- `path`
- `old_text`
- `new_text`
- optional `all`

Hard-fail contract includes:

- `"old_text must not be empty."`
- `"File not found: ${path}"`

### 2.2 Patch mode schema

- `path`
- `op` (`create | delete | update`, unknown values normalized to `update`)
- optional `rename`
- optional `diff`

Hard-fail contract includes:

- `"Diff contains no hunks"`
- multi-file patch rejection (see parser section)

### 2.3 Hashline mode schema

- `path`
- `edits[]` where each edit has:
  - `op` (`replace|append|prepend`)
  - `pos` anchor (`LINE#ID`) optional
  - `end` anchor optional
  - `lines` (`string[] | string | null`)
- optional file controls:
  - `delete: boolean`
  - `move: string`

---

## 3) Shared normalization and safety layers

Across modes, the engine uses shared normalization:

- BOM strip + preserve (`stripBom`)
- line-ending normalize to LF for matching (`normalizeToLF`)
- restore original ending on output (`restoreLineEndings`)
- indentation helpers (`adjustIndentation`, tab/space conversions)

Plan-mode and notebook guards are enforced before write:

- `enforcePlanModeWrite(...)`
- rejects `.ipynb` through edit tool (must use notebook tool)

Filesystem cache invalidation is explicit post-write/delete/move:

- `invalidateFsScanAfterWrite`
- `invalidateFsScanAfterDelete`
- `invalidateFsScanAfterRename`

---

## 4) Replace mode deep behavior

### 4.1 Matching pipeline

Replace mode calls `replaceText(...)` (`patch/diff.ts`), which internally:

1. tries exact match first
2. if `all=true`, performs exact global replace fast path
3. if no exact match and fuzzy enabled, iteratively uses `findMatch(...)`
4. for each replacement, adjusts indentation based on matched actual text

Fuzzy/match utilities (`patch/fuzzy.ts`) include:

- normalized similarity (Levenshtein-based)
- dominant fuzzy selection rules
- ambiguity counting and occurrence previews

### 4.2 Ambiguity and failure behavior

If multiple occurrences found, operation fails with contextual preview and disambiguation guidance.

If no match, throws `EditMatchError` with:

- closest match location
- similarity percent
- first differing old/new line snippet
- threshold/fuzzy guidance

### 4.3 Post-match write path

- applies writethrough (`#writethrough`) for LSP integration
- computes display diff via `generateDiffString`
- includes diagnostics metadata in `details.meta`

---

## 5) Patch mode deep behavior

Patch mode uses `applyPatch(...)` in `patch/applicator.ts` with a `FileSystem` abstraction.

### 5.1 Operation semantics

#### create

- requires `diff` content
- normalizes create content via `normalizeCreateContent` (strips leading `+` diff additions when needed)
- ensures trailing newline
- creates parent dirs as needed

#### delete

- file must exist
- captures old content
- deletes file

#### update

- requires `diff`
- file must exist
- parses hunks via `parseHunks(...)`
- applies hunk sequence with fuzzy/context matching
- supports rename/move on update

### 5.2 Diff normalization/parsing

`normalizeDiff(...)` (`patch/parser.ts`) strips wrappers/metadata safely:

- codex wrappers: `*** Begin Patch`, `*** End Patch`
- codex file markers: `*** Update/Add/Delete File:`
- unified diff metadata: `diff --git`, `index`, `---`, `+++`, mode/rename markers

It intentionally preserves actual diff content lines (` ` / `+` / `-`), preventing accidental data loss.

### 5.3 Multi-file guard

`parseHunks(...)` rejects multi-file diffs using marker counting:

- >1 file marker => hard fail
- single-file patches only

### 5.4 Hunk formats accepted

`parseOneHunk(...)` accepts:

- empty marker: `@@` / `@@ @@`
- unified header: `@@ -old,count +new,count @@ context`
- context marker: `@@ something`
- line hint forms: `@@ line 125`, `@@ lines 12-15`
- nested `@@` anchors for hierarchical context

Also supports EOF marker `*** End of File` inside hunk.

### 5.5 Matching and placement algorithm

Core logic in `computeReplacements(...)`:

- validates line hints (`>=1`, range validity)
- resolves hierarchical context via `findHierarchicalContext(...)`
- uses sequence matching (`seekSequence`) with strategy progression:
  - exact
  - trim-trailing
  - trim
  - comment-prefix
  - unicode
  - prefix
  - substring
  - fuzzy
  - character fallback
- allows hint-window disambiguation (`chooseHintedMatch`) near line hints
- uses fallback hunk variants when needed:
  - `trim-common`
  - `dedupe-shared`
  - `collapse-repeated`
  - `single-line`

### 5.6 Ambiguity and overlap hard-fail behavior

Hard fail categories include:

- ambiguous context matches (includes strategy hint and previews)
- ambiguous sequence matches (multiple candidate indices)
- overlapping replacements across hunks
- unresolved expected lines even after fallback search

This is critical: ambiguity is not “best effort” applied silently; it is fail-fast.

### 5.7 Character-match fast path

`applyHunksToContent(...)` enables a char-level fast path only for narrow conditions:

- single hunk
- no context marker
- no context lines
- no oldStartLine hint
- not EOF-targeted

Otherwise uses full replacement computation pipeline.

### 5.8 Newline policy and content restoration

After edits:

- preserves original trailing newline behavior
- restores original line ending style
- restores BOM if present

### 5.9 Patch warnings propagation

Warnings from application (e.g., dominant fuzzy decisions) are merged into diagnostics path and surfaced in result metadata.

---

## 6) Hashline mode deep behavior

Hashline mode (`patch/hashline.ts`) is integrity-first line-addressed editing.

### 6.1 Identity model

Line identity: `LINE#ID`

- `LINE`: 1-indexed line number
- `ID`: 2-char hash from custom nibble alphabet
  - `NIBBLE_STR = "ZPMQVRWSNKTXJBYH"`
  - hash from `Bun.hash.xxHash32` over whitespace-normalized line text
  - punctuation-only lines mix line number seed to reduce collisions

Formatted read display:

- `LINENUM#HASH:TEXT`

### 6.2 Anchor parse and validation

`parseTag(...)` parses and validates references; invalid refs fail immediately.

Before mutation, `applyHashlineEdits(...)` pre-validates all refs:

- out-of-range line -> fail
- hash mismatch -> collect mismatch
- if any mismatch, throw `HashlineMismatchError` containing:
  - all mismatches
  - contextual preview with `>>>` markers
  - computed remap table (expected -> current)

No partial writes occur on stale anchors.

### 6.3 Resilient anchor mapping

`resolveEditAnchors(...)` maps flat tool payload to structural ops:

- replace with `pos+end` => range replace
- replace with one anchor => single-line replace
- append/prepend can use `pos` or `end`
- file-level append/prepend allowed without anchors

### 6.4 Pre-application transforms and warnings

Hashline mode performs guardrail transforms:

- escaped tab indentation autocorrection (`PI_HASHLINE_AUTOCORRECT_ESCAPED_TABS`)
- suspicious unicode placeholder warning for `\uDDDD`
- duplicate edit deduplication
- trailing replacement line duplicate auto-removal for range replaces

### 6.5 Mutation ordering and no-op tracking

Edits are sorted bottom-up by effective line to keep anchors stable.

It tracks `noopEdits` when replacement is identical to existing content, feeding user-facing diagnostics (`No changes made ...`).

### 6.6 Hashline operation semantics

- `replace`: single-line or inclusive range replacement
- `append`: insert after anchor or EOF when no anchor
- `prepend`: insert before anchor or BOF when no anchor

Supports file delete/move via parent edit-tool layer.

---

## 7) Preview computation and renderer coupling

Preview helpers (`patch/diff.ts`):

- `computeEditDiff(...)`
- `computePatchDiff(...)`
- `computeHashlineDiff(...)`

Used by `ToolExecutionComponent` for pre-execution visualization.

Renderer (`patch/shared.ts`) supports:

- streaming diff previews
- streaming hashline edit previews
- hunk/line truncation in collapsed mode
- file/lang metadata and line-count headers
- diagnostics rendering
- `mergeCallAndResult = true` behavior

---

## 8) LSP writethrough integration

Edit tool is wired to LSP-aware write callbacks:

- `createLspWritethrough(...)` used when LSP + diagnostics/format settings enabled
- batch coordination through `getLspBatchRequest(...)`
- patch mode uses `LspFileSystem` adapter to route writes through writethrough
- flush behavior for batched write operations to produce final diagnostics

Rust rewrite must preserve the writethrough abstraction boundary:

- edit engine should not know LSP internals
- filesystem adapter should remain pluggable for diagnostic-producing writes

---

## 9) Error taxonomy and parity-critical strings

Important error classes:

- `ParseError`
- `ApplyPatchError`
- `EditMatchError`
- `HashlineMismatchError`

Observed parity-critical strings (must remain compatible for tests/operators):

- `"old_text must not be empty."`
- `"Diff contains no hunks"`
- `"File not found: ${path}"`

Ambiguity errors intentionally include previews and “add more context” guidance.

---

## 10) Rust rewrite requirements (edit subsystem)

### 10.1 Keep three mode engines distinct

Implement separate modules with shared primitives:

- `replace_engine`
- `patch_engine`
- `hashline_engine`
- shared `normalization`, `fuzzy`, `diff_render`, `error_types`

Avoid one giant polymorphic function; parity-critical behavior is mode-specific and complex.

### 10.2 Preserve pre-mutation full validation

Especially for hashline and patch ambiguity:

- validate all anchors/line hints before mutation
- detect overlap and ambiguity before applying any change

### 10.3 Preserve strict fail-fast ambiguity policy

Do not silently pick one of many plausible matches. Matching must either:

- uniquely resolve, or
- fail with contextual diagnostics

### 10.4 Preserve newline/BOM and indentation behavior

These are user-visible and diff-visible semantics, not implementation details.

### 10.5 Preserve dry-run preview architecture

Patch and AST-style flows rely on preview-before-apply patterns and renderer integration.

---

## 11) Suggested Rust module structure for edit subsystem

- `edit/mod.rs` (mode router + dynamic mode resolution)
- `edit/replace.rs`
- `edit/patch/parser.rs`
- `edit/patch/apply.rs`
- `edit/hashline.rs`
- `edit/fuzzy.rs`
- `edit/normalize.rs`
- `edit/diff.rs`
- `edit/render_bridge.rs`
- `edit/errors.rs`
- `edit/fs_adapter.rs` (plain FS + LSP writethrough adapter)

---

## 12) Subsystem acceptance checks for migration

1. **Mode routing parity**
   - same env/settings/model precedence for edit mode selection.
2. **Replace ambiguity parity**
   - same behavior on zero/one/multiple matches.
3. **Patch parser parity**
   - same acceptance/rejection for wrappers, metadata, and hunk forms.
4. **Patch ambiguity parity**
   - same fail behavior for ambiguous matches and overlap.
5. **Hashline staleness parity**
   - same mismatch detection and remap/context output model.
6. **Line ending/BOM parity**
   - no unintended content-style drift.
7. **Renderer preview parity**
   - pre-execution diff visibility and collapsed/expanded truncation behavior.

This subsystem is one of the highest-risk areas in the rewrite because it sits directly on file mutation correctness.
