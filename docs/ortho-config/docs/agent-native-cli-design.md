# Agent-native CLI assistance design

## 1. Purpose and scope

This document defines how OrthoConfig will help Rust projects build
agent-native command-line interfaces. It translates the agent-native CLI
principles accepted in the planning conversation into product requirements,
metadata contracts, lint policy, and roadmap boundaries.

The design is intentionally stricter than a documentation guideline. The
project goal is to make agent-native behaviour mechanically visible and
enforceable through derive metadata, generated context, and `cargo-orthohelp`
checks. Maintainers should not have to rely on code review to catch a command
that uses `info` where the rest of the CLI uses `get`, accepts
`--skip-confirmations` instead of `--force`, or emits a broad table where an
agent needs JSON to be bounded.

This document covers the product shape and the future implementation contract.
It does not claim that all features are already implemented. The implementation
order lives in [roadmap.md](roadmap.md).

## 2. Product rationale

OrthoConfig already asks developers to describe configuration once and derive
the CLI, environment, file, merge, and documentation behaviour from that
description. That makes it the right place to assist with agent-native CLI
contracts, because the hard problem is consistency across a whole command
surface.

The product direction is:

- describe command, option, output, and workflow contracts once;
- generate human documentation from documentation-oriented metadata;
- generate compact agent invocation context from agent-oriented metadata;
- lint command vocabulary and behavioural gaps before release;
- make `cargo-orthohelp` the reference CLI for the first agent-native tier.

The design preserves OrthoConfig's current identity. OrthoConfig is not being
turned into a mandatory application runtime. It should model, generate,
validate, and optionally provide reusable helpers for common surfaces such as
profiles, delivery targets, feedback stores, and job ledgers. Downstream
applications still own their domain side effects.

This document is the canonical agent-native contract and boundary reference for
OrthoConfig. Other project documents should link here when they need to
describe agent-native scope instead of duplicating the full contract.

## 2.1 Consumer application boundary

Weaver and Netsuke are the first planned consumers that make the boundary
concrete. OrthoConfig owns the reusable command-contract machinery:

- schemas and command metadata;
- documentation IR and compact agent-context IR;
- vocabulary and global-option policy;
- renderer metadata for human and machine output;
- generated help, man pages, completions, and reference artefacts;
- policy linting and drift checks;
- optional primitives for profiles, delivery targets, feedback stores, skill
  manifests, and execution ledgers.

Weaver owns semantic execution: capability routing, Rope, rust-analyzer,
Language Server Protocol (LSP), Tree-sitter, and Sempai providers, sandboxing,
Double-Lock safety, actual edits, job execution, semantic refusal logic, and
provider-specific idempotency.

Netsuke owns build and package semantics: manifest interpretation, subprocess
execution, build graph logic, package-specific run records, and any domain
payloads it sends to delivery or feedback sinks.

This split lets Weaver and Netsuke depend on OrthoConfig for consistent command
contracts without pushing their execution engines into a configuration crate.
OrthoConfig models, generates, serializes, and lints reusable command
contracts. Downstream applications own command execution, side effects,
domain-specific safety policy, and long-running job semantics.

The consumer dependency tier for each reusable capability is defined in §2.2,
which is the authoritative source for hard and soft ship-time dependencies.

## 2.2 Consumer dependency tier

This section records ship-time contract dependency, not runtime resilience and
not delivery state. A ship-time hard dependency means Weaver's generated
command surface cannot ship without the named OrthoConfig capability. A
ship-time soft dependency means a consumer may keep shipping by carrying a
temporary local adapter while OrthoConfig support arrives later. This framing
is aligned with the hard and soft dependency language used by the
[AWS Well-Architected Reliability Pillar](https://docs.aws.amazon.com/wellarchitected/latest/reliability-pillar/),
but inside OrthoConfig it is only a ship-time contract rule. Do not reuse
these words in this project to mean runtime fallback, circuit breaking, or any
other resilience pattern.

The tier is independent of delivery status. A capability is hard or soft based
on whether Weaver's generated command surface can ship without the reusable
OrthoConfig contract, irrespective of whether that contract is already
implemented, in flight, or planned for a later roadmap item. When a
soft-dependency consumer adapter shadows an OrthoConfig roadmap item, it must
declare the shadowed item so replacement can be tracked. Once OrthoConfig
publishes the reusable contract, the consumer must replace the local adapter in
the next consumer release. If the local adapter and published OrthoConfig shape
disagree, the published OrthoConfig shape wins.

Table 1 is the dependency-tier matrix. If a back-reference elsewhere in the
documentation set disagrees with this table, this table wins.

| Capability                | Tier | Roadmap item                                                                                                                           | Consumer adaptation rule                                                                                            | Replacement trigger                                                                                      |
| ------------------------- | ---- | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| Whole-CLI introspection   | Hard | §6.1 "Populate subcommand metadata"                                                                                                    | No local adaptation is permitted for Weaver's generated command surface.                                            | Not applicable.                                                                                          |
| Strict vocabulary policy  | Hard | §7.1 "Implement vocabulary policy"                                                                                                     | No local adaptation is permitted for Weaver's generated command surface.                                            | Not applicable.                                                                                          |
| Agent-context IR          | Hard | §6.2 "Add compact agent-context output"                                                                                                | No local adaptation is permitted for Weaver's generated command surface.                                            | Not applicable.                                                                                          |
| Localized help generation | Hard | Existing `OrthoConfigDocs` and `OrthoConfigSubcommandDocs` contracts with §6.1.1 "Generate recursive `DocMetadata.subcommands` values" | No local adaptation is permitted for Weaver's generated command surface.                                            | Not applicable.                                                                                          |
| Profiles                  | Soft | §9.1 "Profile contracts" and design §6.7                                                                                               | A consumer may carry a temporary `--profile` parsing adapter only.                                                  | Replace within the next consumer release once §9.1 ships; on shape conflict, the OrthoConfig shape wins. |
| Delivery                  | Soft | §9.2 "Delivery and feedback contracts" and design §6.8                                                                                 | A consumer may carry a temporary `--deliver` parsing adapter for `stdout`, `file:<path>`, and `webhook:<url>` only. | Replace within the next consumer release once §9.2 ships; on shape conflict, the OrthoConfig shape wins. |
| Feedback                  | Soft | §9.2 "Delivery and feedback contracts" and design §6.8                                                                                 | A consumer may carry a temporary `feedback <text>` parsing adapter that writes local JSONL only.                    | Replace within the next consumer release once §9.2 ships; on shape conflict, the OrthoConfig shape wins. |
| `skill_manifests`         | Soft | §6.3 "Validate skill manifests against real commands" and design §3.4                                                                  | A consumer may carry a temporary local manifest parser only.                                                        | Replace within the next consumer release once §6.3 ships; on shape conflict, the OrthoConfig shape wins. |
| Execution ledgers         | Soft | §9.3 "Execution ledger contracts" and design §6.6                                                                                      | A consumer may carry a temporary local JSONL ledger record format only.                                             | Replace within the next consumer release once §9.3 ships; on shape conflict, the OrthoConfig shape wins. |

Table 1: consumer dependency-tier matrix.

Soft dependencies permit a temporary adapter for the reusable contract shape,
such as parsing `--profile`, `--deliver`, `feedback <text>`, or a JSONL ledger
record. They do not permit a consumer to fork the agent-context schema,
documentation IR, or policy-report schema permanently. They also do not move
domain execution into OrthoConfig or allow OrthoConfig contracts to duplicate
consumer domains: Weaver still owns semantic code editing and Netsuke still
owns build graph execution and package-specific run records.

## 3. Contract surfaces

Agent-native support is split across four surfaces. Each surface has a
different audience and stability requirement.

### 3.1 Human documentation IR

The existing `OrthoConfigDocs` and `DocMetadata` model remains the source for
localized documentation, roff man pages, PowerShell help, and generated
reference material. It is allowed to contain prose, headings, examples, and
localization metadata.

The documentation IR must continue to be versioned and serializable. It should
gain only the command metadata required to keep human documentation accurate.
It must not become the compact agent context by accident.

### 3.2 Agent context

The new agent-context contract is a compact, machine-oriented JSON document
that describes how to invoke the CLI. It is not localized prose and should be
kept small enough for agents to load cheaply.

The documentation IR and agent-context schema are sibling outputs from the same
metadata spine, not nested versions of one another. The documentation IR stays
localized and human-documentation-oriented. The agent-context schema stays
compact, machine-oriented, and independently versioned, so agent-facing changes
do not force documentation IR migrations unless the same data is genuinely
needed by both outputs.

Schema ownership is defined in
[ADR-0003](adrs/adr-0003-define-schema-ownership-for-agent-native-contracts.md).
Reusable agent-context types and `ORTHO_AGENT_CONTEXT_SCHEMA_VERSION` live in
`ortho_config::agent_context`. `cargo-orthohelp` may generate or transform the
context in later roadmap work, but it does not own the reusable context
contract. The documentation IR remains owned by `ortho_config::docs`, so Fluent
identifiers, localized long prose, roff details, and PowerShell help structures
are not agent-context source fields.

The `cargo-orthohelp` generator interface is:

```console
cargo orthohelp --format agent-context
```

This remains a generator format, not the downstream application command name.
It writes `agent-context.json` and emits the same compact JSON document to
stdout, followed by a newline. Downstream applications expose the public
command surface `context --json`:

```console
example-cli context --json
```

The payload below is illustrative and hand-authored; a generated payload may
include different optional fields or field ordering.

```json
{
  "schema_version": "1",
  "kind": "example-cli.agent_context"
}
```

This keeps the public command approachable while preserving an explicit machine
schema. Downstream applications do not ship a public `agent-context` alias
before the first public release, unless a migration explicitly requires one.
Compatibility detection uses `schema_version`; consumers must not parse `kind`
as a version. ADR-0007 records the naming decision and prior-art divergence.

The top-level shape should include:

```json
{
  "schema_version": "1",
  "package": "example-cli",
  "commands": [],
  "profiles": {
    "supported": false
  },
  "feedback": {
    "supported": false
  },
  "policy": {
    "agent_native": "warn"
  }
}
```

Each command entry should include the invocation path, canonical verb, input
flags, value types, required flags, defaults, enum values, output modes,
pagination controls, mutation boundaries, interaction mode, async metadata,
delivery support, and examples that are short enough for an agent to use
directly.

### 3.3 Agent-native lint policy

The lint policy is the enforcement layer. It is exposed through
`cargo-orthohelp` and is reusable by tests or continuous integration.

The policy-report schema is initially owned by `cargo_orthohelp::policy`,
including `ORTHO_POLICY_REPORT_SCHEMA_VERSION`. This keeps warnings, hard
failures, source locations, rule identifiers, machine-readable codes, and mode
handling close to the reference CLI that emits them. A later ADR can extract a
shared report model if downstream libraries need to construct identical reports.

The implemented command is:

```console
cargo orthohelp --check-agent-native [--policy-mode <off|warn|deny>]
```

The enforcement mode comes from `[package.metadata.ortho_config.policy]` and
defaults to `off`; `--policy-mode` overrides it for the report and requires the
check flag. The check resolves the package manifest without the bridge build,
so it also runs for a package that has no `root_type` or library target.

`cargo orthohelp --check-agent-native` emits one compact, machine-stable policy
report to stdout, followed by a newline, and writes the same report atomically
to `policy-report.json` in the output directory. It also prints a short human
summary to standard error. Tests and CI should parse `rule_id` and `code` for
deterministic handling; prose in `message` is explanatory and may improve
without changing the machine contract. If an explicit generator format is
requested at the same time, the policy report owns stdout and remains the
invocation's only JSON document. A successful check continues into generation
and retains the selected artefacts, including `agent-context.json` for
`--format agent-context` and `--format all`; AgentContext JSON is suppressed
from stdout while policy checking is enabled. A deny-mode finding is reported
before the command fails, so generator artefacts are not created.

<!-- markdownlint-disable MD013 -->
```json
{
  "version": "1",
  "tool": "cargo-orthohelp",
  "mode": "warn",
  "results": [
    {
      "rule_id": "agent-native.config.redundant-exception",
      "severity": "warn",
      "code": "redundant_exception",
      "message": "exception for verb 'get' is redundant: the item is already canonical",
      "location": null
    }
  ],
  "summary": {
    "off": 0,
    "warn": 1,
    "deny": 0,
    "total": 1
  },
  "exceptions": [
    {
      "kind": "verb",
      "name": "get",
      "reason": "redundant but part of the migration surface",
      "command_path": null
    }
  ],
  "vocabulary": {
    "verbs": ["get", "list", "create", "update", "delete", "jobs", "profile", "feedback"],
    "flags": [
      "--json", "--no-input", "--force", "--dry-run", "--limit", "--cursor",
      "--wait", "--profile", "--deliver"
    ]
  }
}
```
<!-- markdownlint-enable MD013 -->

Each result must contain:

- `rule_id`: stable policy rule identifier;
- `severity`: one of `off`, `warn`, or `deny`;
- `code`: stable machine-readable finding code;
- `message`: human-readable diagnostic text;
- `location`: optional nested source location object whose `file` is the
  repository-relative or package-relative path, and whose optional `range`
  holds one-based `start`/`end` positions (matching the shipped
  `cargo_orthohelp::policy` types rather than the draft's flat `file`/`range`
  pair). The current evaluator carries no source spans, so its findings use
  `null`.

Mode handling is direct: `off` suppresses checks, `warn` emits findings without
failing the command, and `deny` returns a policy-validation failure with
process status 1 when any deny-level finding is present. That status is shared
with generic tool failures, so `summary.deny` is the authoritative CI signal.
Clap usage errors keep exit code 2.

The configuration surface is `[package.metadata.ortho_config.policy]` in the
target package's `Cargo.toml` (see ADR-0008). The enforcement default is `off`
(the check is opt-in and checks nothing without a policy table); the
agent-context advertisement default remains the shipped `warn` value. Both
defaults are distinct, and the transient `--policy-mode` override affects only
the report's effective mode, never the generated agent context. Configured
exceptions are visible in the report with their reasons and in agent context
without reasons (see ADR-0008).

### 3.4 Long-form workflow material

Long-form skill manifests, tutorials, or MCP-facing descriptions are useful,
but they are downstream of the compact contracts. They should be generated or
validated against the documentation IR and agent context rather than maintained
as an independent source of truth.

Skill manifests are still first-class contracts. OrthoConfig models only the
manifest path, schema version, and command index. Downstream applications
perform manifest validation and own skill-specific prose and workflow guidance,
such as Weaver's safe Rust rename workflow or Netsuke's build workflow.

## 4. Whole-CLI introspection

Whole-CLI introspection is the first implementation dependency. The
documentation IR already has recursive `subcommands`, and
`OrthoConfigSubcommandDocs` now lets generated `OrthoConfigDocs`
implementations populate that tree from `clap::Subcommand` enums. This closes
the human-documentation IR gap needed before compact agent-context output can
reuse the same command tree.

The nested-command behavioural fixture suite closes the verification gate for
this layer: it asserts recursive metadata, behavioural scenarios, renderer
snapshots, and an end-to-end `cargo-orthohelp` bridge smoke test before any
agent-context format consumes the tree.

The target is a command tree where each command node can describe:

- its command path, such as `profile save` or `jobs get`;
- its canonical verb, such as `get`, `list`, `create`, `update`, or `delete`;
- accepted flags and positional arguments;
- whether it reads, writes, deletes, or submits work;
- whether it can prompt and which flag bypasses prompting;
- output contracts and stable exit classes;
- pagination, async, delivery, profile, and feedback support.

`SelectedSubcommandMerge` and `OrthoConfigSubcommandDocs` share clap command
name parsing, so command trees are generated from Rust types rather than
manually copied into documentation.

## 5. Vocabulary and flag policy

Agent-native CLIs should use vocabulary that neighbouring CLIs already teach
agents to expect. OrthoConfig should therefore support a strict vocabulary
policy that is opt-in at first and capable of becoming a project default later.

The canonical command verbs are:

- `get` for one resource;
- `list` for collections;
- `create` for creation;
- `update` for mutation;
- `delete` for destructive removal;
- `jobs` for durable async job inspection;
- `profile` for persistent identity configuration;
- `feedback` for reporting friction to maintainers.

The canonical flags are:

- `--json` for structured output;
- `--no-input` for non-interactive operation;
- `--force` for destructive prompt bypass;
- `--dry-run` for previewing consequential operations;
- `--limit` and `--cursor` for bounded list output;
- `--wait` for blocking until an async submission completes;
- `--profile` for selecting a named persistent identity;
- `--deliver` for routing generated artefacts.

The canonical downstream introspection command is `context --json`, defined by
[ADR-0007](adrs/adr-0007-downstream-context-command-naming.md). The `context`
command name is reserved for application surfaces, while
`cargo-orthohelp --format agent-context` remains the build-time generator
format.

The canonical human-facing global option glossary is:

- `--color auto|always|never` for colour policy;
- `--emoji auto|always|never` for emoji policy when a project supports emoji;
- `--progress auto|always|never` for progress and spinner policy;
- `--accessibility auto|on|off` for accessibility-oriented rendering;
- `--plain` for plain text fallback;
- `--no-pager` for pager suppression;
- `--width <columns>` for terminal-width-sensitive rendering;
- `--locale <locale>` for localized human output;
- `--quiet` and `--verbose` for diagnostic verbosity.

The policy should flag or reject off-convention aliases such as `info`, `ls`,
`--format=json`, `--output json`, and `--skip-confirmations` when strict mode
is enabled. Projects may still opt out or configure exceptions, but exceptions
must be explicit and visible in generated context. It should also flag
near-miss global options when a project uses legacy names for a concept in the
glossary, such as `--output-format`, `--colour-policy`, `--diag-json`, boolean
`--progress`, `--no-emoji`, or boolean `--accessible`.

## 6. Required command semantics

Agent-native metadata must describe behaviour, not only syntax. A flag list is
not enough for an agent to decide whether an invocation is safe, bounded, or
recoverable.

### 6.1 Non-interactive execution

Commands should run without prompts when invoked by an agent. Metadata should
state whether a command is non-interactive, may prompt, or requires a bypass
flag. A command that may prompt without declaring `--force`, `--yes`,
`--no-input`, or an equivalent project-approved bypass should fail strict
agent-native lint.

The preferred non-interactive flag is `--no-input`. The preferred destructive
bypass flag is `--force`. If a project chooses a different convention, it must
configure that convention once and expose it in agent context.

This is realized in the derive attribute surface as
`behaviour(interaction = ...)` with the optional `behaviour(bypass = ...)`
flag, and in agent context as `interaction_mode` plus `bypass_flag`. See
[ADR-0010](adrs/adr-0010-behavioural-metadata-attribute-surface.md) and the
§8.1 table below for the defaulting and compatibility contract.

### 6.2 Structured output

Data-returning commands should support `--json`. Structured data belongs on
stdout, diagnostics belong on stderr, and exit codes should be stable enough to
document.

Agent-context output metadata should describe:

- whether `--json` is supported;
- whether stdout contains data, a path, a summary, or no output;
- whether diagnostics can appear on stderr;
- stable exit classes and their meanings;
- the response schema when it is known.

`cargo-orthohelp` should dogfood this by returning structured success summaries
for generated artefacts when `--json` is provided.

JSON mode must be stricter than "mostly JSON". The default contract is:

- success writes exactly one JSON result document to stdout and nothing to
  stderr;
- failure writes no stdout unless an explicit artefact has already been
  delivered, and exactly one JSON diagnostic document to stderr;
- subprocess output never leaks beside the result document on stdout;
- protocol identifiers and error classes are not localized.

Some commands legitimately need streams. OrthoConfig should therefore model a
`JsonModeContract`-style shape with these choices:

- `success_stdout`: `one_json_document`, `jsonl_stream`, or `artifact_path`;
- `failure_stderr`: `one_json_diagnostic` or `jsonl_diagnostics`;
- `subprocess_output_policy`: `capture_preview_and_log`,
  `inherit_only_in_human_mode`, or `forbidden`.

The lint policy should reject commands that claim JSON mode but cannot state
where every byte of stdout and stderr goes.

### 6.2.1 Renderer metadata

OrthoConfig should model renderer metadata without becoming every downstream
renderer. The reusable contract should describe:

- human renderer support and machine renderer support;
- TTY sensitivity and closed-stdin behaviour;
- colour, emoji, progress, pager, width, accessibility, and plain-output
  policy;
- stdout and stderr contracts per renderer;
- localized versus non-localized fields;
- whether progress or subprocess output may be inherited in human mode only.

This supports Weaver's dual human/machine output and Netsuke's human-first,
agent-consistent presentation from the same metadata vocabulary.

### 6.2.2 Exit-code taxonomy metadata

OrthoConfig should model exit codes in documentation IR and agent context, but
it should not impose a universal taxonomy. A command contract should be able to
describe:

```json
{
  "exit_codes": {
    "0": { "class": "success" },
    "2": { "class": "usage" },
    "5": { "class": "external_tool_failure" }
  }
}
```

Strict policy should lint that every documented error class has an exit code,
and that JSON diagnostics report the class and code consistently.

### 6.3 Errors that enumerate choices

When a command rejects a value because it is outside a known set, the error
should name the valid set. This applies to enum values, output formats,
delivery schemes, locale identifiers, package names, binary targets, profile
names, and policy modes.

Roadmap item 5.1.1 reconciled the missing-required-values design with the
actual error enum. The current public surface does not expose
`OrthoError::MissingRequiredValues`; missing required values still route
through the existing command-line parsing, merge, gathering/deserialization, or
aggregate error channels. The improved aggregate diagnostic remains phase 7.3.1
implementation work.

### 6.4 Mutation boundaries and retries

Mutating commands should declare whether they are read-only, write, delete, or
submit asynchronous work. Destructive commands should declare their
confirmation bypass flag. Consequential commands should declare whether
`--dry-run` exists.

This is realized in the derive attribute surface as `behaviour(mutation = ...)`
with the optional `behaviour(dry_run = ...)` flag, and in agent context as
`mutation_effect` plus `dry_run_flag`. See
[ADR-0010](adrs/adr-0010-behavioural-metadata-attribute-surface.md) for the
attribute grammar and the no-inference rule.

Create-like commands should prefer idempotency tokens or natural keys where the
application domain supports them. OrthoConfig should model and lint the
contract; the downstream application owns the domain-specific idempotency store.

### 6.5 Bounded responses

List-shaped commands should declare bounded defaults. The canonical flags are
`--limit` and `--cursor`. Metadata should include the default limit, maximum
limit where applicable, cursor support, and whether truncation hints are
returned.

Generated agent descriptions should also be bounded. The agent-context schema
should include concise summaries rather than long prose copied from man pages.

### 6.6 Async-aware execution

OrthoConfig should distinguish asynchronous configuration loading from
application-level async jobs. The agent-native requirement concerns
submit-poll-collect workflows in downstream CLIs.

Async submit commands should declare:

- a `--wait` flag;
- the response field that contains the job identifier;
- status commands, using the public noun configured by the project;
- whether a durable execution ledger exists;
- whether retries can recover an in-flight job.

The generic concept is an execution ledger. The public noun is configurable, so
Weaver can expose `jobs` while Netsuke can expose `runs` without forking the
metadata model. Ledger metadata should cover record identifiers, status enums,
timestamps, command paths, input hashes, idempotency keys, log references,
result references, prune commands, and bounded list behaviour.

OrthoConfig may later provide reusable helper types for execution-ledger
metadata or storage, but the initial design requirement is to model and lint
the command contract.

### 6.7 Persistent profiles

Profiles let agents reuse named bundles of configuration across invocations.
Profile support should be optional, but when present it should use a canonical
root flag:

```console
example-cli render --profile weekly-recap --json
```

The recommended precedence is:

```text
built-in defaults < config files < selected profile < environment < flags
```

If implementation work decides that profiles are named config overlays, the
roadmap must document the exact merge order and migration impact before code is
changed.

That documentation requirement is satisfied by
[ADR-0012](adrs/adr-0012-profile-selection-and-layering.md), recorded before
any behavioural code under roadmap 9.1.1. Profiles are named config overlays:
`[profile.<name>]` tables inside the resolved configuration file chain,
selected statelessly by a `--profile <name>` flag with a `<PREFIX>PROFILE`
environment-variable fallback. The exact merge order is:

```text
built-in defaults < config files < selected profile < environment < flags
```

Profile support is opt-in via `#[ortho_config(profiles)]`; derives that do not
opt in keep the four-tier order and keep reporting
`profiles: { "supported": false }`. An explicitly provided flag beats the
selected profile even when the flag value equals the built-in default. The
migration impact, including the Rust-level `AgentContext.profiles` retype and
the rollback story, is recorded in ADR-0012's migration plan.

Agent context should expose whether profiles are supported, how to list them,
which flag selects one, and which profile fields are redacted or
reference-only. Profile metadata should support secret redaction, profile names
in context output, and generated profile documentation.

### 6.8 Delivery and feedback

Two-way I/O has two separate contracts:

- `--deliver` routes generated artefacts to `stdout`, `file:<path>`, or
  `webhook:<url>`;
- `feedback <text>` records local friction reports and optionally sends them
  upstream when an endpoint is configured.

Unknown delivery schemes should produce a structured refusal that enumerates
the supported schemes. File delivery should use atomic writes. Webhook delivery
should surface HTTP status and retryability.

Feedback should write a local JSONL record by default. If an upstream endpoint
is configured, the command should report whether the local record was also sent
successfully.

OrthoConfig owns parsing, validation, schema metadata, and enumerating errors.
It does not own the domain payload sent by Weaver or Netsuke.

### 6.9 Capability and provenance metadata

Capability-hidden providers are a reusable command-contract shape. OrthoConfig
should model generic capability and provenance metadata such as:

```json
{
  "capability_id": "symbol.rename",
  "command": "symbols rename",
  "kind": "actuator",
  "provider_visibility": "provenance",
  "provider_override": "advanced",
  "provenance_in_json": true
}
```

The generic metadata belongs in OrthoConfig because many CLIs hide backend
providers behind stable user intent. Provider registries, provider selection,
semantic execution, and safety harnesses remain application-owned. Strict
policy should be able to warn when provider names are required in ordinary
public commands instead of being exposed as provenance or advanced override
metadata.

## 7. `cargo-orthohelp` as reference CLI

`cargo-orthohelp` should be the first CLI in the workspace to satisfy the
table-stakes agent-native behaviours:

- non-interactive operation by default;
- `--json` for structured command results;
- generated artefact summaries on stdout in JSON mode;
- diagnostics on stderr;
- enumerating errors for invalid formats, packages, binaries, locales, and
  policy modes;
- stable exit classes documented in its README;
- atomic writes for generated files;
- agent-native lint and agent-context output from the compiled metadata.

This gives downstream users an executable reference rather than only a design
document.

## 8. Versioning and compatibility

The documentation IR and agent-context schema must version independently. A
change that affects man-page generation may not affect agents, and a compact
agent-context addition should not force a documentation IR migration unless the
same data is genuinely needed by human documentation.

### 8.1 Defaulting for legacy derives

Older derives will not emit every new metadata field immediately. The
agent-context schema, documentation IR, and man-page generation must therefore
apply explicit defaults instead of guessing from absent data.

The table distinguishes fields realized in agent-context schema v1 from
forward-looking fields planned for later schema versions. Readers for schema v1
apply defaults only to realized fields; planned rows record the intended future
contract and do not imply that those fields exist today.

| Field                   | Default                  | Status  | Rationale                                                                  |
| ----------------------- | ------------------------ | ------- | -------------------------------------------------------------------------- |
| `canonical_verb`        | `null`                   | v1      | Legacy command metadata did not classify verbs.                            |
| `supports_json`         | `false`                  | planned | Structured output must be declared before tools rely on it.                |
| `json_stdout_contract`  | `null`                   | planned | No JSON stream invariant exists until the command opts in.                 |
| `json_stderr_contract`  | `null`                   | planned | Diagnostics remain unspecified for legacy commands.                        |
| `exit_classes`          | `[]`                     | planned | Exit-code semantics are unavailable unless documented.                     |
| `interaction_mode`      | `"unknown"`              | v1      | Legacy derives cannot prove whether a command prompts.                     |
| `mutation_effect`       | `"unknown"`              | v1      | Read/write/delete boundaries must not be inferred from names.              |
| `pagination`            | `null`                   | v1      | List bounds and cursors require explicit command metadata.                 |
| `profiles.supported`    | `false`                  | v1      | Profiles are opt-in persistent state.                                      |
| `profiles.selection`    | `null`                   | v1      | Selection contract exists only once a derive opts into profiles.           |
| `profiles.list_command` | `null`                   | v1      | Profile listing commands arrive with the 9.1.3 store helpers.              |
| `delivery_route`        | `null`                   | v1      | Delivery sinks change artefact routing and must be explicit.               |
| `feedback.supported`    | `false`                  | v1      | Feedback storage or upload must be explicitly available.                   |
| `execution_ledger`      | `{ "supported": false }` | planned | Jobs, runs, or tasks require application-owned execution state.            |
| `skill_manifests`       | `[]`                     | v1      | Skill manifests are absent until declared; validation lands in 6.3.2.      |
| `capability_id`         | `null`                   | planned | Capability routing is optional downstream metadata.                        |
| `provider_provenance`   | `{ "reported": false }`  | planned | Provider names are not emitted unless the application declares provenance. |
| `renderer.human`        | `{ "supported": true }`  | planned | Existing documentation IR already supports human help material.            |
| `renderer.machine`      | `{ "supported": false }` | planned | Machine renderer support must be declared before agents depend on it.      |

In roadmap 7.1.1, `PolicyInputs` is empty and the evaluator does not inspect
bridge metadata. Omitted bridge metadata therefore produces no `warn` or `deny`
findings. Bridge-metadata findings and their mode-dependent behaviour are
planned for roadmap 7.1.2. Once those rules exist, omitted metadata will follow
the selected mode: `off` suppresses checks, `warn` reports findings without
failing the command, and `deny` fails CI with validation diagnostics.

The first implementation phase should introduce agent-native support behind
explicit formats, commands, or metadata attributes. Existing generated
documentation output should remain compatible unless the roadmap names a
migration step.

Legacy defaulting is an OrthoConfig reader or generator responsibility. JSON
Schema `default` annotations may document the intended value, but validation
does not mutate older payloads or fill missing fields. A consumer that reads
older documentation IR or agent-context JSON must therefore apply the defaults
above in its deserialization or transform layer before it relies on the
metadata.

New metadata fields must not be inferred from command names, option spelling,
or absent values. In particular, OrthoConfig does not guess mutation effects,
JSON output support, interaction mode, exit classes, pagination, profile
support, capability provenance, delivery support, feedback support, or
execution-ledger support. The default is always the least capable compatible
state until a derive attribute, application adapter, or later schema version
declares a stronger contract.

The existing `cargo-orthohelp --format ir`, `--format man`, `--format ps`,
`--format agent-context`, and `--format all` outputs are compatibility
surfaces. Issue #329 adds stdout JSON for the agent-native formats while
preserving their generated artefact paths and exit-status behaviour; policy
report output takes precedence when combined with an explicit generator format.
Other output formats and their stdout/stderr contracts remain unchanged.

Strict policy should begin as opt-in. Projects should be able to run the check
in warning mode before enforcing it in CI.

### 8.2 Agent-context schema compatibility policy

The agent-context schema is versioned independently of the documentation IR as
recorded in
[ADR-0003](adrs/adr-0003-define-schema-ownership-for-agent-native-contracts.md).
`ORTHO_AGENT_CONTEXT_SCHEMA_VERSION` is a major-version string for the compact
wire shape emitted by `cargo orthohelp --format agent-context` and by
`--format all`. It is an integer-valued breaking-change counter, not SemVer:
additive changes keep the same value, and breaking changes bump it.

Within a major version, producers may add optional fields or populate a
previously `null` optional field with data that already has a defined meaning.
They may add enum variants only when the contract defines and tests an
unknown-variant fallback that preserves the documented legacy default for
strict deserializers. Readers must ignore unknown object fields, and schema
types must not add `deny_unknown_fields` while retaining the same major
version. Absent fields use the realized-field defaults recorded in §8.1;
validation never mutates payloads to populate those defaults.

Breaking changes require bumping `ORTHO_AGENT_CONTEXT_SCHEMA_VERSION`. Breaking
changes include renaming or removing fields, changing enum wire strings,
changing enum casing, changing required fields, moving fields between objects,
changing a field's JSON type, changing a serialized default or `Default`
implementation, toggling whether an optional field is omitted or serialized as
`null`, adding `deny_unknown_fields`, removing an enum variant, or changing the
meaning of an existing value. The v1 contract deliberately uses snake_case enum
strings.

JSON object ordering is not semantically meaningful to consumers, but the
wire-contract snapshot is byte-sensitive. Field or key reordering therefore
requires a deliberate snapshot review even when the schema meaning does not
change.

`AgentCommand.summary` is intentionally omitted when absent, as are the optional
`ProfilesDeclaration` fields (`selection` and `list_command`) added by roadmap
9.1.1. Other optional schema v1 fields serialize as explicit `null` when
absent. This asymmetry is part of the wire contract; changing it is breaking.

`AgentInput.default` is a best-effort human-readable display. It is useful for
selection and review, but it is not normative, executable, or
machine-parseable. The generator normalizes unstable Rust token spacing around
`::` outside literals before emitting the string, so toolchain formatting
changes do not churn goldens. String and character literal contents remain
unchanged.

The contract is pinned by a byte-exact snapshot and variant-exhaustive
wire-value tests for every enum. When the version is bumped, retain the prior
version's frozen wire fixture and a round-trip or compatibility test, so
overlap compatibility is demonstrable. JSON Schema emission remains a deferred
enhancement; if it is added, it belongs in dev tooling or behind a non-default
feature, not in the runtime schema path.

Schema v1 history:

- `1`: locked by roadmap item 6.2.2. The lock standardizes agent-context enum
  wire strings to snake_case, records the null-versus-omitted optional-field
  policy, and includes agent-context output in `cargo orthohelp --format all`.
- `1` (extended by 9.1.1): `profiles` retyped to `ProfilesDeclaration` at the
  Rust level (a pre-approved pre-1.0 break per
  [ADR-0012](adrs/adr-0012-profile-selection-and-layering.md)); the new
  optional fields `selection` and `list_command` follow the omitted-when-absent
  rule, and the unsupported case remains byte-identical to
  `{ "supported": false }`.

## 9. Current gaps to resolve

The design and roadmap updates must address these known gaps:

- the improved `MissingRequiredValues` diagnostic is reconciled as proposed
  phase 7 work, but is not yet implemented;
- `cargo-orthohelp` has no structured `--json` result mode;
- `cargo-orthohelp` does not yet enumerate all valid choices in errors;
- generated file writes are not specified as atomic;
- renderer metadata, JSON-mode stream contracts, exit-code taxonomy metadata,
  skill manifest validation, capability/provenance metadata, profile redaction,
  delivery, feedback, execution ledger, mutation, and pagination metadata are
  not yet modelled as first-class contracts.

The following gaps block ordinary `clap` command surfaces from being described
at all, and are therefore prerequisites rather than enhancements. See
[cargo-orthohelp-design.md](cargo-orthohelp-design.md) §§2.2, 3.1, 3.6, 4.2.1,
and 12 for the intended contracts:

- unit subcommand variants and named-field subcommand variants are rejected by
  the documentation derive, which excludes both service-style command sets and
  the canonical clap idiom for three-level command trees;
- positional arguments are unrepresentable: the IR models only long and short
  flags, so `<url>`-style and `<src> <dst>`-style commands cannot be described
  or emitted into agent context;
- `OrthoConfigDocs` cannot be derived independently of `OrthoConfig`, so
  documenting a type also requires making it loadable from configuration
  layers, which excludes most subcommand argument structs;
- the documentation IR and agent-context types are neither `#[non_exhaustive]`
  nor constructible without struct literals, so no metadata field can be added
  without a breaking release;
- `Unknown` enum variants exist for forward compatibility but no unknown-value
  fallback is wired to them, so §8.2's additive-variant allowance cannot be
  used;
- no default heading catalogue ships with the library, so every adopter
  reimplements the same eleven identifiers before generated documentation is
  translatable.

## 10. Deferred extensions

The following ideas are useful but must not block the core agent-native work:

- MCP server generation from agent context;
- OpenAPI-shaped runtime explorer endpoints for downstream applications;
- remote configuration providers;
- live reload of configuration;
- managed application execution ledgers;
- asynchronous configuration file loading.

These extensions should be revisited only after whole-CLI introspection,
agent-context output, strict lint policy, and the `cargo-orthohelp` reference
CLI are working.
