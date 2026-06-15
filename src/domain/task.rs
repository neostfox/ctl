use crate::domain::approval::{ApprovalState, ApprovalStatus};
use crate::domain::event::Event;
use crate::domain::lease::{LeaseState, LeaseStatus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Planning,
    Ready,
    InProgress,
    Review,
    Completed,
    Cancelled,
}

impl Phase {
    /// Canonical machine string form (serde `snake_case`), matching the
    /// `phase` field written to `task.json` and the schema enum. This is the
    /// single source of truth for the wire/string form — do NOT derive phase
    /// strings from `format!("{:?}", ..)` (which yields `inprogress`, an
    /// incompatible spelling that silently breaks gate matching).
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Planning => "planning",
            Phase::Ready => "ready",
            Phase::InProgress => "in_progress",
            Phase::Review => "review",
            Phase::Completed => "completed",
            Phase::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Phase::Planning => write!(f, "Planning"),
            Phase::Ready => write!(f, "Ready"),
            Phase::InProgress => write!(f, "In Progress"),
            Phase::Review => write!(f, "Review"),
            Phase::Completed => write!(f, "Completed"),
            Phase::Cancelled => write!(f, "Cancelled"),
        }
    }
}
/// Outcome of running a required gate.
///
/// Frozen protocol: each gate retains only the latest result.
/// The completion interlock requires all gates to have `passed: true`
/// before `task_completed` can be emitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateResult {
    /// Identifier matching a gate in the task definition.
    pub gate_id: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Evidence description (command output summary, hash, etc.).
    pub evidence: String,
    /// ISO 8601 timestamp of when the gate was checked.
    pub checked_at: String,
    /// Git tree hash this gate result was validated against (artifact binding).
    /// `None` for legacy events recorded before tree binding existed; such
    /// unbound results cannot satisfy the finish-time artifact interlock.
    #[serde(default)]
    pub tree_hash: Option<String>,
    /// Canonical policy hash in force when this gate ran (policy binding).
    /// `None` for legacy events; unbound results cannot satisfy a new finish.
    #[serde(default)]
    pub policy_hash: Option<String>,
}

impl fmt::Display for GateResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.passed { "PASS" } else { "FAIL" };
        write!(f, "{}: {} ({})", self.gate_id, status, self.evidence)
    }
}

/// Active run information tracked by the task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunInfo {
    pub run_id: String,
    pub adapter: String,
    pub lease_id: String,
    pub started_at_seq: i64,
}

impl fmt::Display for RunInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Run({}) adapter={} lease={}",
            self.run_id, self.adapter, self.lease_id
        )
    }
}
/// Reference to an active agent run, used by M6 multi-agent scheduling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunRef {
    pub run_id: String,
    pub worktree_path: String,
    pub lease_id: String,
}

// ── BS-provenance V1 ────────────────────────────────────────────────────────
//
// Records which cognitive artifacts a task derived from (originator divergence/
// convergence, and a critic artifact or an explicit skip). This is *record-only*
// provenance: it never gates task creation or completion, and it makes no claim
// about thinking quality or review independence. Two invariants are enforced by
// the reducer (not merely by convention), so the immutable ledger can never be
// made to overstate trust:
//   - trust_level is always `content_l0` — recording a reference never raises trust.
//   - critic_independence is always `unattested` — there is no independent
//     orchestrator, so independence can never be recorded as established.

/// Pinned trust level for every brainstorm artifact reference. Bare L0 content.
pub const BRAINSTORM_TRUST_LEVEL: &str = "content_l0";
/// Pinned critic-independence disclosure. V1 has no independent orchestrator.
pub const CRITIC_INDEPENDENCE_UNATTESTED: &str = "unattested";

/// Disposition of the critic (independent-challenge) step for a brainstorm.
///
/// Discloses whether a critic artifact was attached, the step was explicitly
/// skipped, or neither happened — it never evaluates the critic's quality.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CriticDisposition {
    /// Neither a critic artifact nor a skip decision has been recorded.
    Absent,
    /// A critic artifact was attached.
    Present,
    /// The critic step was explicitly skipped, with a recorded reason and actor.
    Skipped,
}

impl CriticDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            CriticDisposition::Absent => "absent",
            CriticDisposition::Present => "present",
            CriticDisposition::Skipped => "skipped",
        }
    }
}

/// A content artifact bound by path and SHA-256 hash at the moment it was
/// recorded. Both fields are L0 content: the binding fixes *what* was referenced,
/// not that the content is trustworthy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactRef {
    pub path: String,
    pub hash: String,
}

/// Provenance reference linking a task to the brainstorm it derived from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrainstormRef {
    pub id: String,
    pub divergence: Option<ArtifactRef>,
    pub convergence: Option<ArtifactRef>,
    pub critic: Option<ArtifactRef>,
    pub critic_disposition: CriticDisposition,
    /// Always `unattested` in V1 (see module note). The reducer rejects any
    /// attempt to record a different value.
    pub critic_independence: String,
    /// Always `content_l0` in V1 (see module note). The reducer rejects any
    /// attempt to record a higher trust level.
    pub trust_level: String,
    /// Claimed originating run id, if any. Never attested in V1 — its presence
    /// records a claim, not a verified provenance link.
    pub source_run_id: Option<String>,
    /// Actor that recorded the originator artifacts.
    pub recorded_by: String,
    /// Why the critic step was skipped (set only when disposition is `Skipped`).
    pub skip_reason: Option<String>,
    /// Who decided to skip the critic (set only when disposition is `Skipped`).
    pub skip_decided_by: Option<String>,
}

/// Display status of one recorded artifact, computed against the file on disk.
/// `present` means the file still exists; `stale` means it is missing or its
/// current hash no longer matches what was recorded.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArtifactStatus {
    pub path: String,
    pub present: bool,
    pub stale: bool,
    pub recorded_hash: String,
}

/// A fact-only rendering of a task's brainstorm provenance, with staleness
/// resolved against the current working tree. Deliberately carries no pass/fail
/// verdict and no independence claim — it discloses, it does not evaluate.
#[derive(Debug, Clone, Serialize)]
pub struct BrainstormProvenanceView {
    pub id: String,
    pub divergence: Option<ArtifactStatus>,
    pub convergence: Option<ArtifactStatus>,
    pub critic: Option<ArtifactStatus>,
    pub critic_disposition: String,
    /// Always `unattested` in V1.
    pub critic_independence: String,
    /// Always `content_l0` in V1.
    pub trust_level: String,
    pub source_run_id: Option<String>,
    /// Always `false` in V1 — a recorded source run id is a claim, never attested.
    pub source_run_attested: bool,
    pub recorded_by: String,
    pub skip_reason: Option<String>,
    pub skip_decided_by: Option<String>,
}

// ── Uncertainty Ledger V1 ───────────────────────────────────────────────────
//
// A single record-and-disclose object for the unknowns a task carries. It never
// gates create/finish and never renders an aggregate verdict — it makes the
// remaining uncertainty visible and sourced. Two invariants are enforced by the
// reducer (not merely by convention):
//   - trust_level is always `content_l0` — the statement/source are unverified
//     content; recording them never raises trust.
//   - a disposition is terminal — once an uncertainty is resolved / accepted as
//     an assumption / invalidated, a second disposition is rejected, so an
//     assumption can never be silently upgraded to resolved.
// `resolved` requires hash-bound evidence; `accepted_as_assumption` and
// `invalidated` must NOT carry evidence (they remain unresolved by external
// evidence). `evidence_ref` reuses `ArtifactRef`: ctl computes its hash, so the
// control layer can derive freshness without ever asserting the content is true.

/// Pinned trust level for every uncertainty event. Bare L0 content.
pub const UNCERTAINTY_TRUST_LEVEL: &str = "content_l0";

/// Lifecycle status of a single uncertainty. `Open` is the only non-terminal
/// state; the three terminal states are reached via a disposition and never left
/// in V1 (terminal-is-terminal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertaintyStatus {
    Open,
    Resolved,
    AcceptedAsAssumption,
    Invalidated,
}

impl UncertaintyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            UncertaintyStatus::Open => "open",
            UncertaintyStatus::Resolved => "resolved",
            UncertaintyStatus::AcceptedAsAssumption => "accepted_as_assumption",
            UncertaintyStatus::Invalidated => "invalidated",
        }
    }
}

/// A single recorded unknown. `evidence_ref` is set only when `Resolved`;
/// `reason` carries the "why" for `Invalidated` (and optional context otherwise).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Uncertainty {
    pub id: String,
    pub statement: String,
    /// Free-text note on where it came from. UNATTESTED — a claim, not provenance.
    pub source: Option<String>,
    pub status: UncertaintyStatus,
    /// Hash-bound evidence artifact, present only when status is `Resolved`. For an
    /// Oracle-V1 resolve this is copied from the referenced evidence so freshness is
    /// derived uniformly; for a legacy inline resolve it is the inline artifact.
    pub evidence_ref: Option<ArtifactRef>,
    /// Oracle V1: the id of the recorded evidence that resolved this uncertainty,
    /// when resolved via `evidence_ref`. None for legacy inline resolves and for
    /// every non-resolved state. Absent in old streams (which replay unchanged).
    #[serde(default)]
    pub evidence_id: Option<String>,
    /// Oracle V1: the oracle kind of the resolving evidence, copied from the
    /// referenced evidence. None for legacy inline resolves (oracle unknown) and for
    /// non-resolved states. A `Model` oracle is advisory, never external proof.
    #[serde(default)]
    pub oracle_kind: Option<OracleKind>,
    /// Why it was invalidated, or optional context on another disposition.
    pub reason: Option<String>,
}

/// Pinned trust level for every evidence event. Bare L0 content — recording an
/// oracle-typed evidence never raises trust or asserts the content is correct.
pub const EVIDENCE_TRUST_LEVEL: &str = "content_l0";

/// What kind of oracle produced a piece of evidence. Fixed enum (no free string, no
/// `other`): a free taxonomy invites labels that pretend to be meaningful. `Model`
/// is ALWAYS advisory (never rendered as fact); `Human` is NOT an authenticated
/// principal. The control layer discloses the kind; it never vouches for the claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OracleKind {
    Deterministic,
    Test,
    Runtime,
    Human,
    Model,
    ExternalAuthority,
}

impl OracleKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OracleKind::Deterministic => "deterministic",
            OracleKind::Test => "test",
            OracleKind::Runtime => "runtime",
            OracleKind::Human => "human",
            OracleKind::Model => "model",
            OracleKind::ExternalAuthority => "external_authority",
        }
    }

    /// True only for `Model`: a model oracle is advisory and must never be rendered
    /// as external proof.
    pub fn is_advisory(&self) -> bool {
        matches!(self, OracleKind::Model)
    }
}

/// A first-class, oracle-typed evidence object an uncertainty can be resolved
/// against. `artifact_ref` is file-backed (ctl-computed hash); `source_ref` is an
/// UNATTESTED free-text locator; `recorded_by` is the envelope actor at record time
/// (an unattested principal — never a separate, forgeable payload field). L0 content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub id: String,
    pub oracle_kind: OracleKind,
    pub source_ref: Option<String>,
    pub artifact_ref: ArtifactRef,
    pub recorded_by: String,
}

/// Freshness of a recorded evidence artifact, derived against the working tree.
/// Discloses only whether the file still matches what was recorded — never
/// whether the evidence content is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceFreshness {
    /// File present and hash matches what was recorded.
    Current,
    /// File present but its hash drifted from what was recorded.
    Stale,
    /// File no longer exists on disk.
    Absent,
}

impl EvidenceFreshness {
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceFreshness::Current => "CURRENT",
            EvidenceFreshness::Stale => "STALE",
            EvidenceFreshness::Absent => "ABSENT",
        }
    }
}

/// Fact-only view of one resolved uncertainty's evidence. `attested` is always
/// `false` in V1 — a recorded hash is a binding, never an attestation of content.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EvidenceView {
    pub path: String,
    pub recorded_hash: String,
    pub freshness: EvidenceFreshness,
    /// Always `false` in V1.
    pub attested: bool,
}

/// Fact-only view of one uncertainty for disclosure.
#[derive(Debug, Clone, Serialize)]
pub struct UncertaintyItemView {
    pub id: String,
    pub statement: String,
    pub status: String,
    pub source: Option<String>,
    pub evidence: Option<EvidenceView>,
    /// Oracle V1: the id of the resolving evidence, when resolved via evidence_ref.
    pub evidence_id: Option<String>,
    /// Oracle V1: the resolving evidence's oracle kind (None for legacy inline).
    pub oracle_kind: Option<String>,
    /// Oracle V1: true iff `oracle_kind` is `model` — advisory, NOT external proof.
    pub advisory: bool,
    pub reason: Option<String>,
}

/// Fact-only breakdown of how many recorded evidences came from each oracle kind.
/// Raw counts only — never a score, ratio, or verdict. `model_advisory` is kept on
/// its own line so a model oracle can never be summed into "external proof".
#[derive(Debug, Clone, Default, Serialize)]
pub struct OracleSourcesView {
    pub deterministic: usize,
    pub test: usize,
    pub runtime: usize,
    pub human: usize,
    pub model_advisory: usize,
    pub external_authority: usize,
}

/// A fact-only rendering of a task's uncertainty ledger: raw per-status counts
/// and the items, each with its source and (for resolved) evidence freshness.
/// Deliberately carries NO aggregate verdict, score, ratio, or progress signal.
#[derive(Debug, Clone, Serialize)]
pub struct UncertaintyLedgerView {
    pub open: usize,
    pub accepted_as_assumption: usize,
    pub resolved: usize,
    pub invalidated: usize,
    /// Always `content_l0` in V1.
    pub trust_level: String,
    /// Oracle V1: per-oracle-kind counts over the task's recorded evidence.
    pub oracle_sources: OracleSourcesView,
    pub items: Vec<UncertaintyItemView>,
}

// ── Research/Spike V1 ───────────────────────────────────────────────────────
//
// A task kind whose completion is defined by evidence + epistemic outcomes, not
// by code. Reuses the Uncertainty Ledger for epistemic outcomes and `ArtifactRef`
// for produced artifacts — no new trust model. Disclosure is fact-only: it never
// renders a verdict, and it deliberately surfaces NO "uncertainties discovered"
// scalar (a rankable integer becomes a covert quality metric, and it is
// manufacturable by recording uncertainties before `start`); "recorded after
// start" is disclosed only as a per-item tag.

/// Pinned trust level for every research artifact reference. Bare L0 content.
pub const RESEARCH_TRUST_LEVEL: &str = "content_l0";

/// Whether a task produces code (implementation) or evidence + epistemic
/// outcomes (research). Set at `task_created`; immutable thereafter. Defaults to
/// `Implementation` so legacy streams (and any absent field) replay unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    Implementation,
    Research,
}

impl TaskKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskKind::Implementation => "implementation",
            TaskKind::Research => "research",
        }
    }
}

/// The kind of a produced research artifact. Fixed enum (no free string, no
/// `other`): a free taxonomy invites labels that pretend to be meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchArtifactKind {
    Findings,
    Experiment,
    Recommendation,
    DesignDraft,
}

impl ResearchArtifactKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResearchArtifactKind::Findings => "findings",
            ResearchArtifactKind::Experiment => "experiment",
            ResearchArtifactKind::Recommendation => "recommendation",
            ResearchArtifactKind::DesignDraft => "design_draft",
        }
    }
}

/// A tracked research artifact bound by path + ctl-computed hash. `source_run_id`
/// is an unattested claim (no trusted orchestrator). L0 content throughout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResearchArtifact {
    pub artifact_ref: ArtifactRef,
    pub kind: ResearchArtifactKind,
    pub source_run_id: Option<String>,
}

/// Fact-only view of one produced research artifact, freshness resolved against
/// the working tree (same primitive as evidence freshness).
#[derive(Debug, Clone, Serialize)]
pub struct ResearchArtifactView {
    pub path: String,
    pub recorded_hash: String,
    pub kind: String,
    pub freshness: EvidenceFreshness,
    pub source_run_id: Option<String>,
    /// Always `false` in V1 — a recorded source run is a claim, never attested.
    pub source_run_attested: bool,
}

/// One uncertainty as disclosed in research output: the fact-only item plus a
/// per-item tag for whether it was recorded after the task started. The tag is a
/// fact, never a rankable subtotal.
#[derive(Debug, Clone, Serialize)]
pub struct ResearchUncertaintyView {
    #[serde(flatten)]
    pub item: UncertaintyItemView,
    pub recorded_after_start: bool,
}

/// Fact-only research-output disclosure: raw per-status counts, the produced
/// artifacts, and the uncertainty items. NO aggregate verdict/score/ratio, and
/// deliberately NO "discovered" count — uncertainty reduction is never a success
/// metric.
#[derive(Debug, Clone, Serialize)]
pub struct ResearchOutputView {
    pub artifacts_produced: usize,
    pub uncertainties_opened: usize,
    pub resolved_with_evidence: usize,
    pub accepted_as_assumptions: usize,
    pub invalidated: usize,
    /// Always `content_l0` in V1.
    pub trust_level: String,
    pub artifacts: Vec<ResearchArtifactView>,
    pub uncertainties: Vec<ResearchUncertaintyView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskState {
    pub id: String,
    pub phase: Phase,
    pub is_held: bool,
    pub is_archived: bool,
    pub objective: Option<String>,
    pub read_scope: BTreeSet<String>,
    pub write_allow: BTreeSet<String>,
    pub write_deny: BTreeSet<String>,
    pub risk_triggers: BTreeSet<String>,
    pub gates: BTreeSet<String>,
    /// M-d: Task IDs that must complete before this task can run (declared
    /// dependency edges). Default empty; absent means no dependencies.
    #[serde(default)]
    pub depends_on: BTreeSet<String>,
    /// Latest gate results keyed by gate_id. Each gate retains only the most recent result.
    pub gate_results: HashMap<String, GateResult>,
    /// M4: Active run (at most one per task).
    pub active_run: Option<RunInfo>,
    /// M6: Active agent runs for multi-agent concurrency.
    #[serde(default)]
    pub active_runs: Vec<RunRef>,
    /// M6: Schedule plan ID if this task is part of a planned schedule.
    #[serde(default)]
    pub schedule_plan_id: Option<String>,
    /// BS-provenance V1: optional reference to the brainstorm artifacts this task
    /// derived from. Absent for tasks that never recorded one (including every
    /// task created before this feature existed) — old streams replay unchanged.
    #[serde(default)]
    pub brainstorm_ref: Option<BrainstormRef>,
    /// Uncertainty Ledger V1: the unknowns this task carries, in record order.
    /// Default empty; absent in old streams, which replay unchanged.
    #[serde(default)]
    pub uncertainties: Vec<Uncertainty>,
    /// Oracle V1: oracle-typed evidence objects recorded on this task, in record
    /// order. A `resolved` disposition may reference one by id. Default empty;
    /// absent in old streams, which replay unchanged.
    #[serde(default)]
    pub evidences: Vec<Evidence>,
    /// Research/Spike V1: whether this task produces code or evidence. Set at
    /// create, immutable. Defaults to `Implementation` for legacy/absent streams.
    #[serde(default)]
    pub task_kind: TaskKind,
    /// Research/Spike V1: tracked research artifacts this task produced, in
    /// record order. Default empty; absent in old streams, which replay unchanged.
    #[serde(default)]
    pub research_artifacts: Vec<ResearchArtifact>,
    /// M4: Capability leases keyed by lease_id.
    pub leases: HashMap<String, LeaseState>,
    /// M4: Pending/approved/denied approval requests keyed by request_id.
    pub pending_approvals: HashMap<String, ApprovalState>,
    pub history: Vec<String>,
    pub last_seq: i64,
    pub processed_commands: HashSet<String>,
}

impl TaskState {
    #[allow(dead_code)]
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            phase: Phase::Planning,
            is_held: false,
            is_archived: false,
            objective: None,
            read_scope: BTreeSet::new(),
            write_allow: BTreeSet::new(),
            write_deny: BTreeSet::new(),
            risk_triggers: BTreeSet::new(),
            gates: BTreeSet::new(),
            depends_on: BTreeSet::new(),
            gate_results: HashMap::new(),
            active_run: None,
            active_runs: Vec::new(),
            schedule_plan_id: None,
            brainstorm_ref: None,
            uncertainties: Vec::new(),
            evidences: Vec::new(),
            task_kind: TaskKind::Implementation,
            research_artifacts: Vec::new(),
            leases: HashMap::new(),
            pending_approvals: HashMap::new(),
            history: Vec::new(),
            last_seq: 0,
            processed_commands: HashSet::new(),
        }
    }
}

struct TaskBoundary {
    objective: String,
    read_scope: BTreeSet<String>,
    write_allow: BTreeSet<String>,
    write_deny: BTreeSet<String>,
    risk_triggers: BTreeSet<String>,
    gates: BTreeSet<String>,
    depends_on: BTreeSet<String>,
}

fn decode_task_boundary(payload: &serde_json::Value) -> Result<TaskBoundary, String> {
    if payload.get("scope").is_some() {
        return Err(
            "Legacy scope is not accepted; use read_scope/write_allow/write_deny/risk_triggers/gates"
                .into(),
        );
    }

    let objective = payload
        .get("objective")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "objective is required and must be a string".to_string())?;
    if objective.is_empty() {
        return Err("objective is required and must not be empty".into());
    }

    Ok(TaskBoundary {
        objective: objective.to_string(),
        read_scope: string_set(payload, "read_scope", true)?,
        write_allow: string_set(payload, "write_allow", true)?,
        write_deny: string_set(payload, "write_deny", false)?,
        risk_triggers: string_set(payload, "risk_triggers", false)?,
        gates: string_set(payload, "gates", true)?,
        depends_on: optional_string_set(payload, "depends_on")?,
    })
}

/// Parse an optional string-array field (M-d `depends_on`): absent → empty set.
/// Unlike `string_set`, a missing key is not an error — keeping dependency-free
/// events (which omit the field) valid and byte-identical to pre-M-d output.
fn optional_string_set(
    payload: &serde_json::Value,
    field: &str,
) -> Result<BTreeSet<String>, String> {
    let Some(value) = payload.get(field) else {
        return Ok(BTreeSet::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{field} must be an array of strings"))?;
    let mut set = BTreeSet::new();
    for item in values {
        let s = item
            .as_str()
            .ok_or_else(|| format!("{field} entries must be strings"))?;
        if !s.is_empty() {
            set.insert(s.to_string());
        }
    }
    Ok(set)
}

fn string_set(
    payload: &serde_json::Value,
    field: &str,
    require_non_empty: bool,
) -> Result<BTreeSet<String>, String> {
    let values = payload
        .get(field)
        .and_then(|value| value.as_array())
        .ok_or_else(|| format!("{field} is required and must be an array"))?;
    if require_non_empty && values.is_empty() {
        return Err(format!("{field} is required and must not be empty"));
    }

    let mut normalized = BTreeSet::new();
    for value in values {
        let item = value
            .as_str()
            .ok_or_else(|| format!("{field} entries must be strings"))?;
        normalized.insert(item.to_string());
    }
    Ok(normalized)
}

/// Require a non-empty string field from an event payload.
fn require_str(payload: &serde_json::Value, field: &str) -> Result<String, String> {
    let s = payload.get(field).and_then(|v| v.as_str()).unwrap_or("");
    if s.is_empty() {
        return Err(format!("{field} is required and must not be empty"));
    }
    Ok(s.to_string())
}

/// Read an optional non-empty string field (absent or empty → None).
fn optional_str(payload: &serde_json::Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Fail closed on any attempt to record a trust level above L0 content. Absent →
/// accepted (the reducer pins it). Present and not `content_l0` → rejected, so the
/// immutable ledger can never be made to overstate provenance trust.
fn check_trust_level(payload: &serde_json::Value) -> Result<(), String> {
    if let Some(level) = payload.get("trust_level").and_then(|v| v.as_str()) {
        if level != BRAINSTORM_TRUST_LEVEL {
            return Err(format!(
                "trust_level '{level}' cannot be recorded; provenance trust is always pinned to \
                 '{BRAINSTORM_TRUST_LEVEL}' (recording a reference never raises trust)"
            ));
        }
    }
    Ok(())
}

/// Decode a (path, hash) artifact pair. When `required`, both must be present and
/// non-empty. Otherwise: both absent → None; exactly one present → error (no half
/// pairs, so a recorded artifact always carries the hash that pins it).
fn decode_artifact(
    payload: &serde_json::Value,
    path_field: &str,
    hash_field: &str,
    required: bool,
) -> Result<Option<ArtifactRef>, String> {
    let path = optional_str(payload, path_field);
    let hash = optional_str(payload, hash_field);
    match (path, hash) {
        (Some(path), Some(hash)) => Ok(Some(ArtifactRef { path, hash })),
        (None, None) if !required => Ok(None),
        (None, None) => Err(format!("{path_field} and {hash_field} are required")),
        _ => Err(format!(
            "{path_field} and {hash_field} must be provided together"
        )),
    }
}

/// Decode `task_kind` from a `task_created` payload. Absent → `Implementation`
/// (legacy default). Unknown value → rejected.
fn decode_task_kind(payload: &serde_json::Value) -> Result<TaskKind, String> {
    match payload.get("task_kind").and_then(|v| v.as_str()) {
        None | Some("implementation") => Ok(TaskKind::Implementation),
        Some("research") => Ok(TaskKind::Research),
        Some(other) => Err(format!(
            "task_created: unknown task_kind '{other}' (implementation | research)"
        )),
    }
}

/// Decode the fixed `oracle_kind` enum. Required; unknown value → rejected (no
/// free string, no `other`).
fn decode_oracle_kind(payload: &serde_json::Value) -> Result<OracleKind, String> {
    let kind = require_str(payload, "oracle_kind")?;
    match kind.as_str() {
        "deterministic" => Ok(OracleKind::Deterministic),
        "test" => Ok(OracleKind::Test),
        "runtime" => Ok(OracleKind::Runtime),
        "human" => Ok(OracleKind::Human),
        "model" => Ok(OracleKind::Model),
        "external_authority" => Ok(OracleKind::ExternalAuthority),
        other => Err(format!(
            "evidence_recorded: unknown oracle_kind '{other}' (deterministic | test | \
             runtime | human | model | external_authority)"
        )),
    }
}

/// Decode the fixed `artifact_kind` enum. Required; unknown value → rejected (no
/// free string, no `other`).
fn decode_research_artifact_kind(
    payload: &serde_json::Value,
) -> Result<ResearchArtifactKind, String> {
    let kind = require_str(payload, "artifact_kind")?;
    match kind.as_str() {
        "findings" => Ok(ResearchArtifactKind::Findings),
        "experiment" => Ok(ResearchArtifactKind::Experiment),
        "recommendation" => Ok(ResearchArtifactKind::Recommendation),
        "design_draft" => Ok(ResearchArtifactKind::DesignDraft),
        other => Err(format!(
            "research_artifact_recorded: unknown artifact_kind '{other}' \
             (findings | experiment | recommendation | design_draft)"
        )),
    }
}

pub fn apply(state: &mut TaskState, event: &Event) -> Result<(), String> {
    // R6: Check task_id BEFORE command_id idempotency (per-task, not global)
    if event.task_id != state.id {
        return Err(format!(
            "Task ID mismatch: event targets {}, state is {}",
            event.task_id, state.id
        ));
    }
    if state.processed_commands.contains(&event.command_id) {
        return Ok(());
    }
    if event.seq <= state.last_seq {
        return Err(format!(
            "Sequence error: received {}, expected > {}",
            event.seq, state.last_seq
        ));
    }
    if state.is_held
        && event.event_type != "hold_exited"
        && event.event_type != "boundary_violation_recorded"
        && event.event_type != "gate_checked"
    {
        return Err(format!("Task {} is held.", state.id));
    }

    match event.event_type.as_str() {
        "task_created" => {
            // R5: Reject duplicate task_created (first event always has last_seq == 0)
            if state.last_seq > 0 {
                return Err("Cannot re-create task: already has events".into());
            }
            let boundary = decode_task_boundary(&event.payload)?;
            state.phase = Phase::Planning;
            state.objective = Some(boundary.objective);
            state.read_scope = boundary.read_scope;
            state.write_allow = boundary.write_allow;
            state.write_deny = boundary.write_deny;
            state.risk_triggers = boundary.risk_triggers;
            state.gates = boundary.gates;
            state.depends_on = boundary.depends_on;
            // Research/Spike V1: kind is fixed at creation and never revised.
            state.task_kind = decode_task_kind(&event.payload)?;
        }
        "task_revised" => {
            if state.phase != Phase::Planning {
                return Err(format!(
                    "Can only revise in Planning, current phase: {:?}",
                    state.phase
                ));
            }
            let boundary = decode_task_boundary(&event.payload)?;
            state.objective = Some(boundary.objective);
            state.read_scope = boundary.read_scope;
            state.write_allow = boundary.write_allow;
            state.write_deny = boundary.write_deny;
            state.risk_triggers = boundary.risk_triggers;
            state.gates = boundary.gates;
            state.depends_on = boundary.depends_on;
        }
        "task_marked_ready" => {
            if state.phase != Phase::Planning {
                return Err("Can only mark ready from Planning".into());
            }
            let missing_objective = state
                .objective
                .as_ref()
                .map(|objective| objective.is_empty())
                .unwrap_or(true);
            if missing_objective
                || state.read_scope.is_empty()
                || state.write_allow.is_empty()
                || state.gates.is_empty()
            {
                return Err(
                    "Missing objective, read_scope, write_allow, or gates for Ready".into(),
                );
            }
            state.phase = Phase::Ready;
        }
        "task_started" => {
            if state.phase != Phase::Ready {
                return Err(format!(
                    "Can only start from Ready, current phase: {:?}",
                    state.phase
                ));
            }
            state.phase = Phase::InProgress;
        }
        "task_submitted_for_review" => {
            if state.phase != Phase::InProgress {
                return Err(format!(
                    "Can only submit for review from InProgress, current phase: {:?}",
                    state.phase
                ));
            }
            state.phase = Phase::Review;
        }
        "task_reopened" => {
            if state.phase != Phase::Review {
                return Err(format!(
                    "Can only reopen from Review, current phase: {:?}",
                    state.phase
                ));
            }
            state.phase = Phase::InProgress;
        }
        "task_completed" => {
            if state.phase != Phase::Review {
                return Err(format!(
                    "Can only complete from Review, current phase: {:?}",
                    state.phase
                ));
            }
            // STATE-012: Completion interlock — all required gates must have
            // a latest passing result before completion.
            for gate_id in &state.gates {
                match state.gate_results.get(gate_id) {
                    Some(result) if result.passed => {}
                    _ => {
                        return Err(format!(
                            "Completion interlock: gate '{}' has no passing result",
                            gate_id
                        ));
                    }
                }
            }
            state.phase = Phase::Completed;
        }
        "task_cancelled" => {
            if state.phase == Phase::Completed || state.phase == Phase::Cancelled {
                return Err(format!(
                    "Cannot cancel from terminal phase: {:?}",
                    state.phase
                ));
            }
            state.phase = Phase::Cancelled;
        }
        "task_archived" => {
            if state.phase != Phase::Completed && state.phase != Phase::Cancelled {
                return Err(format!(
                    "Can only archive from terminal phase, current: {:?}",
                    state.phase
                ));
            }
            state.is_archived = true;
        }
        "hold_entered" => {
            state.is_held = true;
        }
        "hold_exited" => {
            state.is_held = false;
        }
        "boundary_violation_recorded" => {
            state.is_held = true;
        }
        "gate_checked" => {
            // Record a gate execution result. Retains only the latest result per gate_id.
            // Fail-closed: reject missing or empty required fields, reject unknown gate_id.
            let gate_id = event
                .payload
                .get("gate_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if gate_id.is_empty() {
                return Err("gate_checked: gate_id is required and must not be empty".into());
            }
            if !state.gates.contains(gate_id) {
                return Err(format!(
                    "gate_checked: gate '{}' is not declared in task gates",
                    gate_id
                ));
            }
            let passed = event
                .payload
                .get("passed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let evidence = event
                .payload
                .get("evidence")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if evidence.is_empty() {
                return Err(format!(
                    "gate_checked: evidence is required for gate '{}'",
                    gate_id
                ));
            }
            let checked_at = event
                .payload
                .get("checked_at")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if checked_at.is_empty() {
                return Err(format!(
                    "gate_checked: checked_at is required for gate '{}'",
                    gate_id
                ));
            }
            let tree_hash = event
                .payload
                .get("tree_hash")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let policy_hash = event
                .payload
                .get("policy_hash")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            state.gate_results.insert(
                gate_id.to_string(),
                GateResult {
                    gate_id: gate_id.to_string(),
                    passed,
                    evidence: evidence.to_string(),
                    checked_at: checked_at.to_string(),
                    tree_hash,
                    policy_hash,
                },
            );
        }
        "evidence_accepted" => {
            // Validate required fields
            let evidence_id = event
                .payload
                .get("evidence_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if evidence_id.is_empty() {
                return Err("evidence_accepted: evidence_id is required".into());
            }
            let source = event
                .payload
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if source.is_empty() {
                return Err("evidence_accepted: source is required".into());
            }
            // Evidence can be accepted in any phase except terminal states
            if state.phase == Phase::Completed || state.phase == Phase::Cancelled {
                return Err("Cannot accept evidence for terminal task".into());
            }
        }
        "evidence_rejected" => {
            let evidence_id = event
                .payload
                .get("evidence_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if evidence_id.is_empty() {
                return Err("evidence_rejected: evidence_id is required".into());
            }
        }
        // ── M4: Workspace events ──
        "workspace_created" => {
            if state.phase != Phase::InProgress {
                return Err(format!(
                    "Can only create workspace in InProgress, current: {:?}",
                    state.phase
                ));
            }
            let worktree_path = event
                .payload
                .get("worktree_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if worktree_path.is_empty() {
                return Err("workspace_created: worktree_path is required".into());
            }
        }
        "workspace_cleaned" => {
            if state.phase != Phase::InProgress {
                return Err(format!(
                    "workspace_cleaned only valid in InProgress, current: {:?}",
                    state.phase
                ));
            }
            let worktree_path = event
                .payload
                .get("worktree_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if worktree_path.is_empty() {
                return Err("workspace_cleaned: worktree_path is required".into());
            }
        }
        "workspace_diff_computed" => {
            if state.phase != Phase::InProgress {
                return Err(format!(
                    "workspace_diff_computed only valid in InProgress, current: {:?}",
                    state.phase
                ));
            }
            // Diff computed is informational; no state mutation.
            // Validate required arrays exist.
            for field in [
                "files_added",
                "files_modified",
                "files_deleted",
                "high_risk",
            ] {
                if event
                    .payload
                    .get(field)
                    .and_then(|v| v.as_array())
                    .is_none()
                {
                    return Err(format!(
                        "workspace_diff_computed: '{}' must be an array",
                        field
                    ));
                }
            }
        }
        "workspace_applied" => {
            if state.phase != Phase::InProgress {
                return Err(format!(
                    "workspace_applied only valid in InProgress, current: {:?}",
                    state.phase
                ));
            }
            let files = event
                .payload
                .get("files_applied")
                .and_then(|v| v.as_array());
            if files.is_none() {
                return Err("workspace_applied: files_applied must be an array".into());
            }
        }
        // ── M4: Run lifecycle events ──
        "run_started" => {
            if state.active_run.is_some() {
                return Err("Cannot start run: another run is already active".into());
            }
            let run_id = event
                .payload
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let adapter = event
                .payload
                .get("adapter")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let lease_id = event
                .payload
                .get("lease_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if run_id.is_empty() || adapter.is_empty() || lease_id.is_empty() {
                return Err("run_started: run_id, adapter, and lease_id are required".into());
            }
            state.active_run = Some(RunInfo {
                run_id: run_id.to_string(),
                adapter: adapter.to_string(),
                lease_id: lease_id.to_string(),
                started_at_seq: event.seq,
            });
        }
        "run_completed" => {
            if state.active_run.is_none() {
                return Err("Cannot complete run: no active run".into());
            }
            state.active_run = None;
        }
        "run_failed" => {
            // run_failed clears active_run regardless of state
            state.active_run = None;
        }
        // ── M4: Lease events ──
        "lease_created" => {
            let lease_id = event
                .payload
                .get("lease_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if lease_id.is_empty() {
                return Err("lease_created: lease_id is required".into());
            }
            if state.leases.contains_key(lease_id) {
                return Err(format!("Duplicate lease_id: {}", lease_id));
            }
            let run_id = event
                .payload
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let resource_path = event
                .payload
                .get("resource_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let action = event
                .payload
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ttl_seconds = event
                .payload
                .get("ttl_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let max_uses = event
                .payload
                .get("max_uses")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if run_id.is_empty() || resource_path.is_empty() || action.is_empty() {
                return Err("lease_created: run_id, resource_path, and action are required".into());
            }
            if ttl_seconds == 0 || max_uses == 0 {
                return Err("lease_created: ttl_seconds and max_uses must be > 0".into());
            }
            state.leases.insert(
                lease_id.to_string(),
                LeaseState {
                    lease_id: lease_id.to_string(),
                    run_id: run_id.to_string(),
                    resource_path: resource_path.to_string(),
                    action: action.to_string(),
                    ttl_seconds,
                    max_uses,
                    remaining_uses: max_uses,
                    created_at_seq: event.seq,
                    status: LeaseStatus::Active,
                },
            );
        }
        "lease_used" => {
            let lease_id = event
                .payload
                .get("lease_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if lease_id.is_empty() {
                return Err("lease_used: lease_id is required".into());
            }
            let lease = state
                .leases
                .get_mut(lease_id)
                .ok_or_else(|| format!("Unknown lease_id: {}", lease_id))?;
            if lease.status != LeaseStatus::Active {
                return Err(format!("Lease '{}' is not active", lease_id));
            }
            if lease.remaining_uses == 0 {
                return Err(format!("Lease '{}' has no remaining uses", lease_id));
            }
            lease.remaining_uses -= 1;
            if lease.remaining_uses == 0 {
                lease.status = LeaseStatus::Expired;
            }
        }
        "lease_expired" => {
            let lease_id = event
                .payload
                .get("lease_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if lease_id.is_empty() {
                return Err("lease_expired: lease_id is required".into());
            }
            let lease = state
                .leases
                .get_mut(lease_id)
                .ok_or_else(|| format!("Unknown lease_id: {}", lease_id))?;
            lease.status = LeaseStatus::Expired;
        }
        "lease_revoked" => {
            let lease_id = event
                .payload
                .get("lease_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if lease_id.is_empty() {
                return Err("lease_revoked: lease_id is required".into());
            }
            let lease = state
                .leases
                .get_mut(lease_id)
                .ok_or_else(|| format!("Unknown lease_id: {}", lease_id))?;
            lease.status = LeaseStatus::Revoked;
        }
        // ── M4: Approval events ──
        "approval_requested" => {
            let request_id = event
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if request_id.is_empty() {
                return Err("approval_requested: request_id is required".into());
            }
            if state.pending_approvals.contains_key(request_id) {
                return Err(format!("Duplicate approval request_id: {}", request_id));
            }
            let reason = event
                .payload
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let scope = event
                .payload
                .get("scope")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let ttl_seconds = event
                .payload
                .get("ttl_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if reason.is_empty() {
                return Err("approval_requested: reason is required".into());
            }
            if ttl_seconds == 0 {
                return Err("approval_requested: ttl_seconds must be > 0".into());
            }
            state.pending_approvals.insert(
                request_id.to_string(),
                ApprovalState {
                    request_id: request_id.to_string(),
                    reason: reason.to_string(),
                    scope,
                    ttl_seconds,
                    requested_at_seq: event.seq,
                    granted_at_seq: None,
                    status: ApprovalStatus::Pending,
                },
            );
        }
        "approval_granted" => {
            let request_id = event
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if request_id.is_empty() {
                return Err("approval_granted: request_id is required".into());
            }
            let approval = state
                .pending_approvals
                .get_mut(request_id)
                .ok_or_else(|| format!("Unknown approval request_id: {}", request_id))?;
            if approval.status != ApprovalStatus::Pending {
                return Err(format!(
                    "Approval '{}' is not pending (status: {:?})",
                    request_id, approval.status
                ));
            }
            approval.status = ApprovalStatus::Granted;
            approval.granted_at_seq = Some(event.seq);
        }
        "approval_denied" => {
            let request_id = event
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if request_id.is_empty() {
                return Err("approval_denied: request_id is required".into());
            }
            let approval = state
                .pending_approvals
                .get_mut(request_id)
                .ok_or_else(|| format!("Unknown approval request_id: {}", request_id))?;
            if approval.status != ApprovalStatus::Pending {
                return Err(format!(
                    "Approval '{}' is not pending (status: {:?})",
                    request_id, approval.status
                ));
            }
            approval.status = ApprovalStatus::Denied;
        }
        "approval_expired" => {
            let request_id = event
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if request_id.is_empty() {
                return Err("approval_expired: request_id is required".into());
            }
            let approval = state
                .pending_approvals
                .get_mut(request_id)
                .ok_or_else(|| format!("Unknown approval request_id: {}", request_id))?;
            approval.status = ApprovalStatus::Expired;
        }
        // ── M6: Multi-agent scheduling events ──
        "run_scheduled" => {
            // Task is assigned to a schedule plan.
            let plan_id = event
                .payload
                .get("plan_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if plan_id.is_empty() {
                return Err("run_scheduled: plan_id is required".into());
            }
            state.schedule_plan_id = Some(plan_id.to_string());
        }
        "run_launched" => {
            // An agent run has been launched for this task.
            if state.phase != Phase::InProgress {
                return Err(format!(
                    "run_launched only valid in InProgress, current: {:?}",
                    state.phase
                ));
            }
            let run_id = event
                .payload
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let worktree_path = event
                .payload
                .get("worktree_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let lease_id = event
                .payload
                .get("lease_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if run_id.is_empty() || worktree_path.is_empty() || lease_id.is_empty() {
                return Err(
                    "run_launched: run_id, worktree_path, and lease_id are required".into(),
                );
            }
            // Check for duplicate run_id
            if state.active_runs.iter().any(|r| r.run_id == run_id) {
                return Err(format!("Duplicate run_id in active_runs: {}", run_id));
            }
            state.active_runs.push(RunRef {
                run_id: run_id.to_string(),
                worktree_path: worktree_path.to_string(),
                lease_id: lease_id.to_string(),
            });
        }
        "run_merged" => {
            // A completed run's worktree diff has been applied to main workspace.
            let run_id = event
                .payload
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if run_id.is_empty() {
                return Err("run_merged: run_id is required".into());
            }
            // Remove from active_runs
            let idx = state
                .active_runs
                .iter()
                .position(|r| r.run_id == run_id)
                .ok_or_else(|| {
                    format!("run_merged: run_id '{}' not found in active_runs", run_id)
                })?;
            state.active_runs.remove(idx);
        }
        // ── BS-provenance V1: record-only artifact provenance ──
        "brainstorm_artifact_recorded" => {
            let brainstorm_id = require_str(&event.payload, "brainstorm_id")?;
            check_trust_level(&event.payload)?;
            let divergence =
                decode_artifact(&event.payload, "divergence_path", "divergence_hash", true)?;
            let convergence = decode_artifact(
                &event.payload,
                "convergence_path",
                "convergence_hash",
                false,
            )?;
            let source_run_id = optional_str(&event.payload, "source_run_id");
            // One brainstorm per task. Re-recording the same id refreshes the
            // originator artifacts (and preserves any critic disposition);
            // binding a different id is rejected.
            let (critic, critic_disposition, skip_reason, skip_decided_by) =
                match &state.brainstorm_ref {
                    Some(existing) if existing.id != brainstorm_id => {
                        return Err(format!(
                            "brainstorm_artifact_recorded: task already bound to brainstorm '{}'",
                            existing.id
                        ));
                    }
                    Some(existing) => (
                        existing.critic.clone(),
                        existing.critic_disposition.clone(),
                        existing.skip_reason.clone(),
                        existing.skip_decided_by.clone(),
                    ),
                    None => (None, CriticDisposition::Absent, None, None),
                };
            state.brainstorm_ref = Some(BrainstormRef {
                id: brainstorm_id,
                divergence,
                convergence,
                critic,
                critic_disposition,
                critic_independence: CRITIC_INDEPENDENCE_UNATTESTED.to_string(),
                trust_level: BRAINSTORM_TRUST_LEVEL.to_string(),
                source_run_id,
                recorded_by: event.actor.clone(),
                skip_reason,
                skip_decided_by,
            });
        }
        "critic_artifact_attached" => {
            let brainstorm_id = require_str(&event.payload, "brainstorm_id")?;
            check_trust_level(&event.payload)?;
            // Independence can never be claimed in V1: reject any value other
            // than `unattested`, so the ledger never asserts a critic was
            // independent when no independent orchestrator exists.
            if let Some(indep) = event
                .payload
                .get("critic_independence")
                .and_then(|v| v.as_str())
            {
                if indep != CRITIC_INDEPENDENCE_UNATTESTED {
                    return Err(format!(
                        "critic_artifact_attached: critic_independence '{indep}' cannot be \
                         recorded; only '{CRITIC_INDEPENDENCE_UNATTESTED}' is permitted in V1 \
                         (no independent orchestrator)"
                    ));
                }
            }
            let critic = decode_artifact(&event.payload, "critic_path", "critic_hash", true)?;
            let reference = state.brainstorm_ref.as_mut().ok_or_else(|| {
                "critic_artifact_attached: no brainstorm recorded for this task".to_string()
            })?;
            if reference.id != brainstorm_id {
                return Err(format!(
                    "critic_artifact_attached: brainstorm id '{}' does not match recorded '{}'",
                    brainstorm_id, reference.id
                ));
            }
            reference.critic = critic;
            reference.critic_disposition = CriticDisposition::Present;
            reference.critic_independence = CRITIC_INDEPENDENCE_UNATTESTED.to_string();
            // Attaching a critic supersedes any prior skip record.
            reference.skip_reason = None;
            reference.skip_decided_by = None;
        }
        "brainstorm_skipped" => {
            let brainstorm_id = require_str(&event.payload, "brainstorm_id")?;
            check_trust_level(&event.payload)?;
            let skip_reason = require_str(&event.payload, "skip_reason")?;
            let decided_by = require_str(&event.payload, "decided_by")?;
            let reference = state.brainstorm_ref.as_mut().ok_or_else(|| {
                "brainstorm_skipped: no brainstorm recorded for this task".to_string()
            })?;
            if reference.id != brainstorm_id {
                return Err(format!(
                    "brainstorm_skipped: brainstorm id '{}' does not match recorded '{}'",
                    brainstorm_id, reference.id
                ));
            }
            reference.critic = None;
            reference.critic_disposition = CriticDisposition::Skipped;
            reference.critic_independence = CRITIC_INDEPENDENCE_UNATTESTED.to_string();
            reference.skip_reason = Some(skip_reason);
            reference.skip_decided_by = Some(decided_by);
        }
        // ── Uncertainty Ledger V1: record-and-disclose unknowns ──
        "uncertainty_recorded" => {
            let id = require_str(&event.payload, "uncertainty_id")?;
            let statement = require_str(&event.payload, "statement")?;
            check_trust_level(&event.payload)?;
            let source = optional_str(&event.payload, "source");
            if state.uncertainties.iter().any(|u| u.id == id) {
                return Err(format!(
                    "uncertainty_recorded: uncertainty '{id}' already recorded"
                ));
            }
            state.uncertainties.push(Uncertainty {
                id,
                statement,
                source,
                status: UncertaintyStatus::Open,
                evidence_ref: None,
                evidence_id: None,
                oracle_kind: None,
                reason: None,
            });
        }
        // ── Oracle V1: record a first-class, oracle-typed evidence object ──
        "evidence_recorded" => {
            check_trust_level(&event.payload)?;
            let id = require_str(&event.payload, "evidence_id")?;
            let oracle_kind = decode_oracle_kind(&event.payload)?;
            let artifact_ref =
                decode_artifact(&event.payload, "artifact_path", "artifact_hash", true)?
                    .ok_or_else(|| {
                        "evidence_recorded: artifact_path and artifact_hash are required"
                            .to_string()
                    })?;
            let source_ref = optional_str(&event.payload, "source_ref");
            if state.evidences.iter().any(|e| e.id == id) {
                return Err(format!(
                    "evidence_recorded: evidence '{id}' already recorded"
                ));
            }
            state.evidences.push(Evidence {
                id,
                oracle_kind,
                source_ref,
                artifact_ref,
                // recorded_by is the envelope actor — an unattested principal, never
                // a separate forgeable payload field that could contradict the actor.
                recorded_by: event.actor.clone(),
            });
        }
        "uncertainty_disposition_recorded" => {
            let id = require_str(&event.payload, "uncertainty_id")?;
            check_trust_level(&event.payload)?;
            let disposition = require_str(&event.payload, "disposition")?;
            // Two evidence shapes: the legacy inline (path+hash) and the Oracle-V1
            // reference (evidence_ref → a recorded evidence id). They are mutually
            // exclusive on a resolve and resolved against state BEFORE the uncertainty
            // is borrowed mutably.
            let inline_evidence =
                decode_artifact(&event.payload, "evidence_path", "evidence_hash", false)?;
            let evidence_id = optional_str(&event.payload, "evidence_ref");
            let reason = optional_str(&event.payload, "reason");
            let has_any_evidence = inline_evidence.is_some() || evidence_id.is_some();
            // Resolve a referenced evidence to owned values up front (disjoint from the
            // mutable uncertainty borrow). Mutual exclusion enforced here.
            let resolved_via_ref = match &evidence_id {
                Some(eid) => {
                    if inline_evidence.is_some() {
                        return Err(
                            "uncertainty_disposition_recorded: a 'resolved' must carry \
                             EITHER evidence_ref OR inline evidence_path/evidence_hash, never both"
                                .to_string(),
                        );
                    }
                    let ev = state
                        .evidences
                        .iter()
                        .find(|e| &e.id == eid)
                        .ok_or_else(|| {
                            format!(
                                "uncertainty_disposition_recorded: evidence_ref '{eid}' does not \
                             reference a recorded evidence in this task"
                            )
                        })?;
                    Some((eid.clone(), ev.artifact_ref.clone(), ev.oracle_kind))
                }
                None => None,
            };
            let uncertainty = state
                .uncertainties
                .iter_mut()
                .find(|u| u.id == id)
                .ok_or_else(|| {
                    format!("uncertainty_disposition_recorded: unknown uncertainty '{id}'")
                })?;
            // Terminal-is-terminal: a disposed uncertainty cannot be disposed
            // again, so an assumption can never be silently upgraded to resolved.
            if uncertainty.status != UncertaintyStatus::Open {
                return Err(format!(
                    "uncertainty_disposition_recorded: uncertainty '{id}' is already '{}'; \
                     a disposition is terminal in V1",
                    uncertainty.status.as_str()
                ));
            }
            match disposition.as_str() {
                "resolved" => {
                    // resolved is the only disposition closed by external evidence —
                    // either a recorded oracle-typed evidence (preferred) or legacy inline.
                    if let Some((eid, artifact_ref, oracle_kind)) = resolved_via_ref {
                        uncertainty.status = UncertaintyStatus::Resolved;
                        uncertainty.evidence_ref = Some(artifact_ref);
                        uncertainty.evidence_id = Some(eid);
                        uncertainty.oracle_kind = Some(oracle_kind);
                        uncertainty.reason = reason;
                    } else if let Some(artifact_ref) = inline_evidence {
                        // Legacy inline evidence: oracle kind is unknown (predates Oracle V1).
                        uncertainty.status = UncertaintyStatus::Resolved;
                        uncertainty.evidence_ref = Some(artifact_ref);
                        uncertainty.reason = reason;
                    } else {
                        return Err("uncertainty_disposition_recorded: 'resolved' requires \
                             evidence — an evidence_ref to a recorded evidence, or legacy inline \
                             evidence_path + evidence_hash (an unknown closed without external \
                             evidence is not resolved)"
                            .to_string());
                    }
                }
                "accepted_as_assumption" => {
                    // An assumption must remain visibly unresolved by external evidence.
                    if has_any_evidence {
                        return Err("uncertainty_disposition_recorded: \
                             'accepted_as_assumption' must not carry evidence (it remains \
                             unresolved by external evidence); use reason"
                            .to_string());
                    }
                    uncertainty.status = UncertaintyStatus::AcceptedAsAssumption;
                    uncertainty.reason = reason;
                }
                "invalidated" => {
                    // "I was wrong / it no longer applies" has no oracle: a reason,
                    // never evidence, so it cannot masquerade as a proof.
                    if has_any_evidence {
                        return Err("uncertainty_disposition_recorded: 'invalidated' must not \
                             carry evidence; record why in reason"
                            .to_string());
                    }
                    let reason = reason.ok_or_else(|| {
                        "uncertainty_disposition_recorded: 'invalidated' requires a reason"
                            .to_string()
                    })?;
                    uncertainty.status = UncertaintyStatus::Invalidated;
                    uncertainty.reason = Some(reason);
                }
                other => {
                    return Err(format!(
                        "uncertainty_disposition_recorded: unknown disposition '{other}'"
                    ));
                }
            }
        }
        // ── Research/Spike V1: record a tracked research artifact ──
        "research_artifact_recorded" => {
            check_trust_level(&event.payload)?;
            let artifact_ref =
                decode_artifact(&event.payload, "artifact_path", "artifact_hash", true)?
                    .ok_or_else(|| {
                        "research_artifact_recorded: artifact_path and artifact_hash are required"
                            .to_string()
                    })?;
            let kind = decode_research_artifact_kind(&event.payload)?;
            let source_run_id = optional_str(&event.payload, "source_run_id");
            state.research_artifacts.push(ResearchArtifact {
                artifact_ref,
                kind,
                source_run_id,
            });
        }
        _ => return Err(format!("Unknown event type: {}", event.event_type)),
    }

    state.last_seq = event.seq;
    state.processed_commands.insert(event.command_id.clone());
    state.history.push(event.event_id.clone());
    Ok(())
}
