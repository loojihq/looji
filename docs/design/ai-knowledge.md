# AI that knows the project: the knowledge layer

Status: proposal. Nothing here is built yet beyond what "Where we are" describes.

## The goal

AI in looji should answer and act from what the project actually is: its code, its history, its issues, its plans,
and its conversations. A question like "why does login fail for empty passwords?", "what's left before the launch?",
or "who should take this issue?" should get an answer that cites the files, commits, and issues it came from, and
says plainly when they aren't enough.

Beyond answering, AI should do real work in the project's workflow:
- **Plans** that are whole architecture definitions with diagrams.
- **Reviews** that check whether an issue was actually done.
- **Awareness** of code as it changes, all of it fast.

That needs one shared **knowledge layer** that every AI feature uses (Ask AI, Explain issue, Generate with AI,
review notes, and the features below), instead of each feature collecting its own context.

## Where we are

- **Code retrieval is lexical and one-shot.** `looji-symbols` parses the repository with tree-sitter into functions,
  types, and modules (name, path, line range). `SqliteStore::refresh_repo_index` caches that per file by content
  hash. `retrieve_hunks` splits the task's words into terms, scores symbols and paths against them, reads the
  matching lines, and packs them into a token budget. Chat sends the result once with the first prompt.
- **Matching is on words that appear in names.** A question phrased differently from the code ("sign in" vs
  `authenticate`) finds nothing, and the model can't ask for more.
- **Project data isn't used at all.** Issues, descriptions, plans, history, chats, and media never reach the model,
  except the one task a chat is about.
- **Claude Code runs without tools.** It's called with `-p --output-format text`, so the agent that could read the
  repository itself only sees what we paste in.
- **The Anthropic API path has no tool use.** It streams one request and one reply.

## Do we need a vector database?

**Not a separate one, and not first.**

- **Code questions are mostly about names.** Identifiers, error strings, and file paths are where lexical search
  (BM25 over code split into words) plus a symbol graph does better than embeddings. Embeddings blur exact
  identifiers.
- **An agent that can search beats one big retrieval.** Claude does well when it can run `search`, open a file,
  follow a reference, and search again. Several cheap, precise steps beat one fuzzy similarity lookup. This is how
  Claude Code itself works, with grep and file reads and no vector index.
- **Embeddings earn their place for meaning, not names.** They help with natural language that doesn't share words
  with its target: "issues like this one", duplicate detection, matching a question to docs or past chats, and
  ranking code when the words don't match.
- **When we add them, they go in the SQLite we already have.** looji is a single-user desktop app with one database
  file. A vector server (Qdrant, Weaviate, pgvector) would add a process, a port, and a second store to back up and
  migrate, for no gain at this scale. Two options:
  - **`sqlite-vec`.** A SQLite extension that is statically linkable and works with rusqlite. It gives vector
    tables and k-nearest-neighbour queries in SQL. This is the preferred option.
  - **Plain BLOB columns and brute-force cosine in Rust.** Fine up to roughly 100k chunks with int8 vectors, and the
    fallback if the extension causes trouble.
- **Compute embeddings locally by default.** Run a small ONNX model in-process (for example `bge-small-en-v1.5`, 384
  dimensions, or a code-tuned model such as `jina-embeddings-v2-base-code`), through `fastembed`/`ort`. Code then
  never goes to a third party just to be indexed, indexing works offline, and it works for Claude Code users who
  have no API key. Anthropic has no embeddings endpoint. Voyage (`voyage-code-3`) can be an opt-in provider for
  better quality when the user adds a key.

So the answer is **hybrid retrieval**: lexical, symbols, structured project data, and later vectors, all in SQLite,
used by an **agent with tools**.

## Architecture

```
Sources ──► Ingest ──► Chunks + metadata ──► Indexes ──► Retrieval tools ──► Agent loop ──► Answer + citations
 code         incremental    (SQLite)          FTS5        search/read/        provider:        rendered with
 docs         hash-based                       symbols     find/list           Claude Code      links to files,
 git                                           SQL         issue tools         or Anthropic     commits, issues
 issues                                        vectors*                        API
 chats/plans                                   summaries*
```
`*` = later phase.

### 1. Sources

| Source | What we read | Updates when |
| --- | --- | --- |
| Code | Source files the language parsers know, with symbols | File hash changes (as today) |
| Docs | `README*`, `docs/**`, `*.md`, ADRs, top-of-file comments | File hash changes |
| Git | Commit messages, authors, dates, changed files, and blame for ownership | New commits (`git log` since last head) |
| Issues | Title, description, status, priority, estimate, dates, tags, assignee, history | Every save, through the store |
| Conversations | Task chats and the project chat, as question and answer pairs | When a reply finishes |
| Plans | Published and unfinished plans: spec, stories, tasks | On save |
| Media | File names and kinds. Later, text from PDFs and images (OCR) | When attached |

Ignore rules: `.gitignore`, a new `.loojiignore`, built-in excludes (`target/`, `node_modules/`, lockfiles, binaries,
minified files), and **secret patterns**. Files like `.env*`, `*.pem`, `id_rsa*`, and `*secret*` are never indexed or
sent.

### 2. Chunking

Each chunk keeps its source, location, and a stable id, so citations open the right place.

- **Code.** One chunk per symbol from tree-sitter (functions, methods, types, modules). A symbol over about 120
  lines is split at child symbols or blank-line boundaries, with the signature repeated as a header. Each chunk
  carries the file's path, language, the enclosing type or module, and the file's imports, so a method reads in
  context. Code outside any symbol (top-level statements, config) is chunked by about 60-line windows with overlap.
- **Docs.** By heading, up to about 400 tokens, with the heading path ("Install › Linux") as a prefix.
- **Issues.** One record per issue (title, then description, then fields). Long descriptions are split by
  paragraph, and every piece keeps the title.
- **Conversations.** One chunk per question and answer pair, with the issue or project it belongs to.
- **Commits.** One chunk per commit: message and changed paths.

The chunk table is shared: `knowledge_chunks(id, project_id, source_kind, source_ref, path, symbol, start_line,
end_line, content, content_hash, updated_at)`.

### 3. Indexes, all in the one SQLite file

1. **Lexical (FTS5).** An FTS5 table over chunk content and metadata, with a tokenizer that also splits
   `camelCase`, `snake_case`, and paths, so `retrieveHunks`, `retrieve hunks`, and `retrieve_hunks` all match. BM25
   ranking replaces today's hand-written term scores. SQLite ships FTS5, and rusqlite's bundled build can enable it.
2. **Symbol graph.** Extend `looji-symbols` to record references: calls, imports, implementations, and uses of a
   type, as `symbol_refs(from_symbol, to_name, kind, path, line)`. This answers "who calls X", "where is Y used", and
   "what does this module depend on", which similarity search can't.
3. **Structured project data.** Plain SQL over tasks, history, and members. An agent asking "what's overdue for
   Maya" should get a query, not a similarity search.
4. **Vectors (phase 3).** One embedding per chunk in a `sqlite-vec` table keyed by chunk id, computed in the
   background. It is rebuilt only for chunks whose hash changed, and recomputed when the model changes, with the
   model name stored next to the vectors.
5. **Summaries (phase 3).** A cached **repo map** (directory tree with a one-line purpose per directory and key
   files) and lazily computed file summaries. The map is sent with every codebase question, so the agent knows where
   to look first.

### 4. Retrieval: hybrid search with fusion

`search(query, scope, filters, k)` runs each index that applies, then merges the results:

- **Candidates.** FTS5 BM25 top 50, symbol name and fuzzy matches top 20, vector top 50 (once available), and
  structured hits for issue scopes.
- **Fusion.** Reciprocal Rank Fusion (`score = Σ 1 / (60 + rank)`). It needs no score calibration between indexes,
  and it rewards results that several indexes agree on.
- **Boosts.** Small, explainable boosts:
  - Files the current issue already references.
  - Recently changed files.
  - Files owned (by blame) by the issue's assignee.
  - Chunks that open issues link to.
- **Rerank (optional).** A cheap model pass over the top 30 ("rank these by relevance to the question"), used only
  when the question is broad. This is phase 4, behind measurement.
- **Packing.** Dedupe overlapping ranges, merge neighbours in the same file, expand to whole symbols, and fit a token
  budget. This keeps today's `pack_hunks` idea.

Retrieval stays in `looji-domain` (ranking and fusion rules, pure and tested) and `looji-persistence` (SQL and
index I/O), as now.

### 5. The agent: tools, not one paste

Every AI feature runs as a short **agent loop** with read-only tools over the knowledge layer:

| Tool | Does |
| --- | --- |
| `search(query, scope?, k?)` | Hybrid search across code, docs, issues, commits, and chats, with citations |
| `read_file(path, start?, end?)` | Lines from a file in the repository, capped |
| `find_symbol(name)` | Definitions that match, with locations |
| `find_references(symbol)` | Callers, importers, and users, from the symbol graph |
| `list_issues(filter)` | Issues by status, assignee, tag, due, or project |
| `get_issue(id)` | One issue with history, media names, and linked chats |
| `recent_changes(since?, path?)` | Commits and the files they touched |
| `repo_map()` | The cached directory map |

- **Anthropic API provider.** Add tool use to `looji-ai`: a loop that sends `tools`, runs each `tool_use` locally,
  returns `tool_result`, and stops at a final answer or a step limit (about 8 steps, and a token cap). Each step
  streams into the UI as a quiet "Searching code for…" line, so the user sees the work.
- **Claude Code provider.** Run `claude -p` with the repository as its working directory and read-only tools allowed
  (Read, Grep, Glob). Expose the looji tools (issues, history, chats, hybrid search) through a small **MCP server**
  that looji starts over stdio and passes with `--mcp-config`. Claude Code then searches the code with its own tools
  and asks looji for project knowledge. Nothing can write: no Edit, Bash, or Write.
- **Same tool contract for both.** Features don't care which provider runs them.

### 6. Answers people can trust

- **Citations.** Every claim that comes from a source links to it. `path:line` opens the file, `#issue` opens the
  card, and a commit opens its message. The answer view lists "Sources used".
- **Grounding rules in every system prompt.** Prefer the sources. Say "I couldn't find this in the project" rather
  than guess. Name the file or issue when making a claim.
- **What left the machine.** A small "Context sent" disclosure shows which files and issues went to the provider.
  Local embeddings mean indexing sends nothing.
- **Index health.** Settings › AI shows the files indexed, when the index was last updated, its size, and errors
  per file. Indexing runs in the background with progress and never blocks the UI.

## What this makes possible (the "intelligent" part)

Grouped by when it becomes possible:

**With hybrid search and tools (phases 1–2)**
- **Ask AI about the codebase** answers with citations, follows references, and reads the actual code it needs.
- **Explain issue** grounds the explanation in the code the issue touches, related issues, and the commits that
  changed that code.
- **Generate with AI** plans against the real repository: modules that exist, conventions in use, and work already
  tracked, so it doesn't plan duplicates.
- **Natural-language search** in the command palette: "issues about payments", "where do we parse dates".
- **"What changed this week"** digest per project: commits grouped by the issues they touch, and issues that moved.

**With embeddings (phase 3)**
- **Duplicate detection** when creating an issue: "Similar: #12 Fix login timeout (In progress)".
- **Estimates from history**: "Similar finished issues took 3–5h".
- **Suggested assignee** from code ownership (blame) plus who closed similar issues.
- **Related issues** on every expanded card.

**Proactive (phase 4)**
- **Risk notes** when an issue's code changed after its estimate, or a due date is near with no linked commits.
- **Stale issue nudges** and blocker detection ("waiting on #8, which hasn't moved in 9 days").
- **Issue ↔ commit linking**, by mention, branch name, or similarity, so history shows the code that closed an issue.

## Knowing when the code changes

The knowledge layer follows the repository as it changes, without a manual refresh.

- **File watcher.** `notify` watches the repository root and applies the ignore rules. Events are debounced (about
  300 ms) and batched. Only the changed files are re-hashed, re-chunked, and re-indexed (FTS, symbol references,
  and later vectors). A save shows up in search in about two seconds.
- **Polling fallback.** WSL paths (`\\wsl.localhost\...`) and network drives don't deliver file events. For those, a
  cheap mtime scan runs every few seconds while the project is open, and the content hash decides what actually
  changed.
- **Git awareness.** Watch `.git/HEAD` and the refs:
  - **Branch switch.** Re-index by hash difference, which is cheap because unchanged files keep their cached rows.
  - **New commits.** Ingested as `commit` chunks.
  - **Pull.** Treated as a batch of changes.
  Read with `gix`, so no `git` binary is needed.
- **Linking commits to issues.** By `#12` or the issue's short id in the message, by branch names like
  `12-pricing-page`, and later by similarity. Each issue keeps the list of commits that touched it.
- **Change events feed the product:**
  - A quiet "Code changed" mark on a card whose linked code changed since the last look.
  - Reviews re-run when new commits land on an issue that's in review (below).
  - A chat answer that cited code changed since is marked "Sources changed since this answer".
  - The weekly digest.

## Reviews that check the work

Today, `looji-ai` has a strict "is it implemented?" check (`review::check`, with verdicts implemented, partial, or
not started, and evidence required) and review notes (`review::notes`: risks, missing tests, things to check). The
UI doesn't use either yet. The review becomes a real step in the workflow.

**Trigger:** an issue moves to **In review**, or someone presses **Review now**. New commits on an issue already in
review re-run it.

**What the reviewer gets**
- The issue: title, description, and a new **Done when** checklist (acceptance criteria, editable in the edit window,
  and filled in by Generate with AI for planned tasks).
- **The change itself.** The commits linked to the issue since it entered In progress, or the working tree's diff
  against the base branch when nothing is linked yet. Diffs come from `gix`, trimmed to the changed hunks plus their
  enclosing symbols.
- Related code, issues, and the blueprint sections the issue touches, through the knowledge layer and its tools, so
  the reviewer can read callers and tests rather than just the diff.
- **Optional test run (phase 4, opt-in per project).** A test command set in project settings (for example
  `cargo test`, or `npm test -- --related`), run in a subprocess in the repository with a timeout. Its pass/fail
  summary and failing test names go into the review. It never runs without the user turning it on, since it runs the
  project's code.

**What it returns** (one strict JSON shape, parsed like the existing checks):
- **Verdict:** Looks done, Needs work, or Can't tell.
- **Per criterion:** met, not met, or unclear, each with evidence (`path:line`, a test name, or a commit). "Met" is
  never accepted without evidence, as `review::check` already requires.
- **Findings:**
  - Missing or weak tests.
  - Risky changes: migrations, error handling, security-sensitive code, public API changes.
  - Scope drift: changes outside the issue.
  - Conflicts with the blueprint.
- **Suggested follow-ups,** which can become issues with one click.

**How it shows up**
- A review panel on the expanded card and in the task window: verdict chip, the Done-when checklist with ticks and
  evidence links, and the findings.
- Actions: **Move to Done**, **Send back** (moves to In progress with the findings added as notes), or **Re-run**.
- The reviewer never moves an issue by itself. A person decides. Reviews are stored (`task_reviews`: verdict, criteria
  results, findings, commit range, model, created_at), so history shows what was reviewed against what.

## Generate with AI: from an idea to a blueprint

Generate with AI grows from "spec, stories, tasks" into a **project blueprint**: a living set of documents and
diagrams that defines the system, stays linked to the issues, and becomes part of the knowledge base.

**What a blueprint holds**
- Overview: problem, goals, non-goals, users.
- **Architecture:** components, what each owns, how they talk, and the data flow.
- Data model: entities, fields, and relations.
- Interfaces: APIs, events, file formats.
- **Decisions:** short records (context, choice, alternatives, consequences).
- Risks and open questions.
- Milestones, then issues. Each issue is linked to the components it builds and carries its Done-when checklist.

**Diagrams, drawn natively.** The model never writes pixels. It emits diagrams as a constrained JSON graph:
`nodes` (kind, label, group), `edges` (kind, label), `groups`, and the diagram type:
- Component.
- Flow or sequence.
- Entity-relationship.
- Deployment.

looji lays them out with a Rust layered-graph layout and draws them in GPUI, so they follow the theme, stay crisp at
any zoom, and every node can link to its component section, files, or issues. Each diagram also exports as
**Mermaid** text, for READMEs and GitHub, and as SVG or PNG. Images the user attaches (sketches, screenshots,
mockups) sit in sections as media. Wireframes can follow the same approach later: structured boxes and labels,
drawn natively.

**Grounded in what exists.** For a project with a repository, generation starts from the actual system:
- **Actual architecture.** Module and dependency graphs computed from the symbol graph and imports, not guessed.
- The repo map, existing issues, and past decisions.

The blueprint then shows **planned vs actual**. A drift view lists components with no code, code that no component
claims, and dependencies the design doesn't allow. This becomes one more source for reviews.

**Generated as an agent, visibly.** The steps are:
1. Research the repository, issues, and documents with the tools.
2. Propose an outline for the user to adjust.
3. Write the sections in parallel, streamed.
4. Draw the diagrams.
5. Check consistency between sections, diagrams, and issues.
6. Produce the issues.

Each step shows as it runs, and any section can be regenerated or refined on its own ("make the data model support
teams"). Refinement exists today for spec, stories, and tasks, and it extends to every section.

**Where it lives.** A **Docs** view per project, next to Board, List, Roadmap, and Workload. Documents are versioned
(each save keeps the previous text, so changes can be compared and restored), indexed like any other source, and
cited by answers and reviews.

## Fast by design

Targets, measured in the evaluation suite:
- Search: under 50 ms p95 on a 50k-line repository.
- Index update after a save: under 2 s.
- First token of an answer: under 1.5 s with the API provider.
- The UI never waits on indexing or AI.

How:
- **Incremental everywhere.** Content hashes decide all work: re-chunk, re-embed, and re-summarise only what
  changed. A cold start of an indexed project does nothing until a file changes.
- **Background workers with priorities.** Indexing runs off the UI thread on its own SQLite connection (WAL). The
  open project and the files of the expanded card go first, and other projects wait.
- **A separate `knowledge.db`.** Derived data stays out of the main database, so writes don't contend with the UI's
  store and upgrade backups stay small.
- **Prompt caching.** The stable prefix (system rules, tool definitions, repo map, project brief) is cached with
  Anthropic prompt caching. Follow-up questions and agent steps then reuse it, which cuts latency and cost a lot.
- **Models matched to the job.** A fast small model (Haiku) for routing, query rewriting, summaries, and commit
  linking. A stronger model for answers, reviews, and blueprints. The choice stays per feature in settings.
- **Prefetch.** When a card expands, its retrieval runs in the background, so Explain issue starts with its context
  ready. When a review is likely (a card is dragged toward In review), its diff is gathered ahead of time.
- **Parallel tool calls** in the agent loop, and streaming for every step and answer.
- **A warm Claude Code session.** CLI start-up costs seconds per call. Keep one session per project alive through
  the Agent SDK's streaming mode, or reuse a session id, so follow-ups don't pay start-up again.

## Phasing

Each phase ships on its own and is useful by itself.

**Phase 1: knowledge foundation, fast and live (about 2–3 weeks)**
- `knowledge.db` with `knowledge_chunks` and an FTS5 index with an identifier-aware tokenizer. Code chunked by symbol,
  docs by heading, issues and chats as records.
- File watcher with polling fallback and git awareness through `gix`. Commits ingested and linked to issues.
- `search()` with BM25 and boosts. `retrieve_hunks` becomes a thin wrapper over it.
- Secret and ignore rules. Index health in Settings. Evaluation fixtures and retrieval metrics in CI.
- Acceptance: recall@10 ≥ 0.8 on the fixture questions. Search under 50 ms p95. A save appears in search within
  2 s, and re-indexing an unchanged repository does no work.

**Phase 2: the agent, citations, and reviews v1 (about 3 weeks)**
- Tool-use loop for the Anthropic API, with streamed step lines. A stdio MCP server plus read-only tools for Claude
  Code. Prompt caching.
- Symbol references, so `find_references` works.
- Citations, "Sources used", and "Context sent".
- Ask AI, Explain issue, and Generate with AI all on the loop.
- Done-when checklists on issues.
- **Review on move to In review**, from linked commits or the working diff: verdict, per-criterion evidence, findings,
  and the review panel with Move to Done and Send back.
- Acceptance: golden questions at least 85% grounded with correct citations. A review of a seeded fixture change
  marks met and unmet criteria correctly in ≥ 80% of fixture cases.

**Phase 3: embeddings, blueprints, and diagrams (about 3–4 weeks)**
- Local ONNX embeddings, `sqlite-vec`, RRF fusion, and optional Voyage.
- Repo map and file summaries.
- Duplicates, related issues, estimate hints, and suggested assignee.
- **Blueprint generator**: sections, decisions, the Docs view with versions, native diagram rendering with Mermaid,
  SVG and PNG export, and planned vs actual architecture from the symbol graph.
- Acceptance: paraphrase recall@10 up by ≥ 0.1 with no loss on identifier questions. A blueprint for the fixture
  project renders every diagram type, and each node links to a section, issue, or file.

**Phase 4: proactive and deeper reviews (ongoing)**
- Opt-in test runs in reviews. Architecture drift findings in reviews.
- Digests, risk notes, stale and blocker nudges.
- A warm Claude Code session. Rerank where measurement says it helps.
- Wireframes, and text from PDFs and images.

## Evaluation (built in phase 1, used for every change)

- **Fixtures.** A small checked-in sample repository plus seeded issues, and a `questions.json` of about 40 questions
  with the files and issues that should be retrieved. Mix identifier questions ("where is `retrieve_hunks` called"),
  paraphrases ("how do we pick which code to show the model"), and project questions ("what's blocking the launch").
- **Retrieval metrics** as a test: recall@k and MRR per question type, so a ranking change that helps one type and
  hurts another is caught in CI.
- **Answer grading** (run by hand or nightly, not in CI): a model grades each answer as grounded, correct, and
  cites its sources, using `LOOJI_FAKE_AI` and recorded responses for the deterministic parts.
- **Budgets.** Tokens per answer, steps per answer, and time to first token, logged locally per feature.

## Data model sketch

```sql
CREATE TABLE knowledge_chunks (
    id           TEXT PRIMARY KEY,           -- stable: hash of source_ref + location
    project_id   TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    source_kind  TEXT NOT NULL CHECK (source_kind IN ('code','doc','issue','chat','commit','plan')),
    source_ref   TEXT NOT NULL,              -- path, task id, thread id, commit sha
    path         TEXT,
    symbol       TEXT,
    start_line   INTEGER,
    end_line     INTEGER,
    content      TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE VIRTUAL TABLE knowledge_fts USING fts5(content, path, symbol, content='knowledge_chunks', content_rowid='rowid');
CREATE TABLE symbol_refs (project_id TEXT, from_path TEXT, from_symbol TEXT, to_name TEXT, kind TEXT, line INTEGER);
CREATE TABLE issue_commits (task_id TEXT, commit_sha TEXT, linked_by TEXT, PRIMARY KEY (task_id, commit_sha));
CREATE TABLE task_reviews (id TEXT PRIMARY KEY, task_id TEXT, verdict TEXT, criteria_json TEXT, findings_json TEXT,
                           commit_range TEXT, model TEXT, created_at INTEGER);
CREATE TABLE blueprint_docs (id TEXT PRIMARY KEY, project_id TEXT, kind TEXT, title TEXT, body TEXT, version INTEGER,
                             updated_at INTEGER);
CREATE TABLE blueprint_diagrams (id TEXT PRIMARY KEY, doc_id TEXT, kind TEXT, graph_json TEXT, version INTEGER);
-- phase 3
CREATE VIRTUAL TABLE knowledge_vec USING vec0(chunk_id TEXT PRIMARY KEY, embedding int8[384]);
CREATE TABLE knowledge_meta (project_id TEXT PRIMARY KEY, embed_model TEXT, last_commit TEXT, repo_map TEXT);
```

## Decisions to make

1. **Local embeddings by default, Voyage opt-in.** Recommended. Private, offline, and no key needed.
2. **`sqlite-vec` vs brute force.** Recommended: `sqlite-vec`, with brute force kept as the fallback behind the same
   trait.
3. **Claude Code with real read-only tools and an MCP server.** Recommended. It is the strongest path for code
   questions, and we'd be wasting the agent otherwise.
4. **Git history as a source** needs `git` on PATH, or a Rust git library (`gix`). Recommended: `gix`, so it works
   in WSL paths and without git installed.
5. **OCR or PDF text for media.** Deferred to phase 4. File names only until then.
6. **Index location.** In the main database (one file to back up) or a separate `knowledge.db` (easy to delete and
   rebuild, keeps backups small). Recommended: a separate file. It's all derived data, and the upgrade backup
   shouldn't copy hundreds of MB of vectors.
7. **Diagrams: native layout vs a bundled Mermaid renderer.** Recommended: native. Mermaid needs a JavaScript engine,
   and native drawing gives themed, clickable, linkable diagrams. Mermaid stays as an export format.
8. **Running tests in reviews.** Recommended: opt-in per project, with a timeout and output limits, never by default.
9. **Claude Code session reuse** (Agent SDK streaming mode vs `--resume`). This needs a spike in phase 2 to measure
   start-up and stability.

## Risks

- **Index size and time on large repositories.** Mitigated by ignore rules, per-file caps, incremental updates, and
  int8 vectors. Index health shows the cost.
- **Tool loops that wander.** Mitigated by step and token caps, a planning hint in the prompt, and visible steps the
  user can stop.
- **Wrong but confident answers.** Mitigated by citations, grounding rules, "not found" behaviour, and the
  evaluation set.
- **Reviews that rubber-stamp.** Mitigated by requiring evidence for every "met", by "Can't tell" as a real outcome,
  by fixture cases with deliberately incomplete changes, and by never moving an issue automatically.
- **Diagram layouts that tangle on big graphs.** Mitigated by capping nodes per diagram (the model splits large
  systems into views), with groups and layered layout. The user can pin positions, which are saved with the diagram.
- **Secrets leaking into prompts.** Mitigated by secret patterns at ingest. Only chunks are ever sent, never whole
  directories, and "Context sent" shows exactly what went out.
