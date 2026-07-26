use crate::domain::approval::{ApprovalState, ApprovalStatus};
use crate::domain::event::Event;
use crate::domain::lease::{LeaseError, LeaseGrant, LeaseState};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
mod task_reducer;

use task_reducer::{cognitive, lifecycle, run};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Planning,
    Proposed,
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
            Phase::Proposed => "proposed",
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
            Phase::Proposed => write!(f, "Proposed"),
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

    /// True only for `Model`: a model oracle is advisory. It must never be rendered as
    /// external proof, and (oracle-resolution semantics) must not resolve an
    /// uncertainty — the command layer rejects a model-backed `resolved`.
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

/// Attestation V1: a subagent dispatch recorded on the parent task ledger.
/// Record-and-disclose: `role`/`adapter` are host-supplied LABELS (unattested,
/// like `recorded_by`), and the instruction/context/output artifacts are
/// file-backed (ctl-computed hash). ctl records what it was told ran — it does
/// NOT verify what actually ran. L0 content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Dispatch {
    /// Host-supplied subagent role/type label (unattested).
    pub role: String,
    /// Host-supplied adapter label (unattested).
    pub adapter: String,
    /// The run this dispatch belonged to, if any (host-supplied).
    pub parent_run: Option<String>,
    /// sha256-bound instruction artifact, if supplied.
    pub instruction: Option<ArtifactRef>,
    /// sha256-bound context artifact, if supplied.
    pub context: Option<ArtifactRef>,
    /// sha256-bound output artifact, if supplied.
    pub output: Option<ArtifactRef>,
    /// The envelope actor at record time (an unattested principal — never a
    /// forgeable payload field).
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
    /// Oracle-resolution semantics forbids new model-backed resolves at the command
    /// layer, so on current streams this flags only a legacy (pre-rule) model resolve.
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

/// Completion-audit depth (ceremony scheme 6). `Full` (default) runs the complete
/// decay rubric (R1–R6/T1–T6) + Health Score; `Light` runs only the closure
/// checklist (build/test/lint evidence existence + protected-path + scope
/// compliance), skipping the decay scan. Both are reviewer-isolated with a hard
/// verdict — `Light` never relaxes reviewer independence, only rubric breadth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuditTier {
    #[default]
    Full,
    Light,
}

impl AuditTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditTier::Full => "full",
            AuditTier::Light => "light",
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
    pub artifacts_recorded: usize,
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
    /// Ceremony scheme 6: completion-audit depth. Default `Full`; `Light` for
    /// small tasks skips the decay rubric. Set at creation, like task_kind.
    #[serde(default)]
    pub audit_tier: AuditTier,
    /// Research/Spike V1: tracked research artifacts this task produced, in
    /// record order. Default empty; absent in old streams, which replay unchanged.
    #[serde(default)]
    pub research_artifacts: Vec<ResearchArtifact>,
    /// Attestation V1: subagent dispatches recorded on this task, in record order.
    /// Default empty; absent in old streams, which replay unchanged.
    #[serde(default)]
    pub dispatches: Vec<Dispatch>,
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
            audit_tier: AuditTier::Full,
            gate_results: HashMap::new(),
            active_run: None,
            brainstorm_ref: None,
            uncertainties: Vec::new(),
            evidences: Vec::new(),
            task_kind: TaskKind::Implementation,
            research_artifacts: Vec::new(),
            dispatches: Vec::new(),
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

/// Decode `audit_tier` from a `task_created` payload. Absent → `Full` (legacy
/// default). Unknown value → rejected.
fn decode_audit_tier(payload: &serde_json::Value) -> Result<AuditTier, String> {
    match payload.get("audit_tier").and_then(|v| v.as_str()) {
        None | Some("full") => Ok(AuditTier::Full),
        Some("light") => Ok(AuditTier::Light),
        Some(other) => Err(format!(
            "task_created: unknown audit_tier '{other}' (full | light)"
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

/// True when `path` lies within `scope` on a path-SEGMENT boundary — equal to it,
/// or strictly beneath it. Both inputs must be normalized, forward-slash and
/// repo-relative. Segment-boundary matching (rather than a raw string prefix) is
/// what prevents the prefix-escape where a scope of `src/auth` would otherwise
/// admit a sibling like `src/authentication`, or `src` would admit `src2`.
///
/// This is the single source of truth for "is this path inside this scope?": the
/// application layer reuses it so the filesystem write gate, evidence audits, and
/// this pure reducer all agree on exactly the same boundary.
pub fn path_within_scope(path: &str, scope: &str) -> bool {
    let scope = scope.trim_end_matches('/');
    if scope.is_empty() {
        return false;
    }
    path == scope || path.starts_with(&format!("{scope}/"))
}

/// True when two normalized, forward-slash, repo-relative scopes overlap — one
/// contains the other on a segment boundary, or they are equal. Used for
/// mutual-exclusion checks (cross-task lease conflicts, concurrent-schedule write
/// scopes) where `src` and `src2`, or `src/auth` and `src/authn`, must count as
/// DISJOINT. Symmetric and free of false negatives: every true containment is
/// caught by one of the two directions.
pub fn scopes_overlap(a: &str, b: &str) -> bool {
    path_within_scope(a, b) || path_within_scope(b, a)
}

/// Pure write-scope test over an already-normalized, repo-relative path (forward
/// slashes). Mirrors the application-layer write-scope matcher so a research
/// artifact is bound by exactly the boundary the write gate enforces — but
/// without touching the filesystem, keeping the reducer pure for replay.
fn artifact_within_write_scope(
    path: &str,
    write_allow: &BTreeSet<String>,
    write_deny: &BTreeSet<String>,
) -> bool {
    let path = path.replace('\\', "/");
    let matches = |scope: &String| path_within_scope(&path, &scope.replace('\\', "/"));
    write_allow.iter().any(matches) && !write_deny.iter().any(matches)
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
        "task_created" => lifecycle::task_created(state, event)?,

        "task_proposed" => lifecycle::task_proposed(state, event)?,

        "task_revised" => lifecycle::task_revised(state, event)?,

        "task_marked_ready" => lifecycle::task_marked_ready(state, event)?,

        "task_approved" => lifecycle::task_approved(state, event)?,

        "task_started" => lifecycle::task_started(state, event)?,

        "task_submitted_for_review" => lifecycle::task_submitted_for_review(state, event)?,

        "task_reopened" => lifecycle::task_reopened(state, event)?,

        "task_completed" => lifecycle::task_completed(state, event)?,

        "task_cancelled" => lifecycle::task_cancelled(state, event)?,

        "task_archived" => lifecycle::task_archived(state, event)?,

        "hold_entered" => {
            if state.phase == Phase::Completed || state.phase == Phase::Cancelled {
                return Err(format!(
                    "Cannot hold a terminal task (phase: {:?})",
                    state.phase
                ));
            }
            state.is_held = true;
        }

        "hold_exited" => {
            state.is_held = false;
        }

        "boundary_violation_recorded" => {
            if state.phase == Phase::Completed || state.phase == Phase::Cancelled {
                return Err(format!(
                    "Cannot record boundary violation on a terminal task (phase: {:?})",
                    state.phase
                ));
            }
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
        "workspace_created" => run::workspace_created(state, event)?,

        "workspace_cleaned" => run::workspace_cleaned(state, event)?,

        "workspace_diff_computed" => run::workspace_diff_computed(state, event)?,

        "workspace_applied" => run::workspace_applied(state, event)?,

        "run_started" => run::run_started(state, event)?,

        "run_completed" => run::run_completed(state, event)?,

        "run_failed" => run::run_failed(state, event)?,

        "lease_created" => run::lease_created(state, event)?,

        "lease_used" => run::lease_used(state, event)?,

        "lease_expired" => run::lease_expired(state, event)?,

        "lease_revoked" => run::lease_revoked(state, event)?,

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
        // ── BS-provenance V1: record-only artifact provenance ──
        "brainstorm_artifact_recorded" => cognitive::brainstorm_artifact_recorded(state, event)?,

        "critic_artifact_attached" => cognitive::critic_artifact_attached(state, event)?,

        "brainstorm_skipped" => cognitive::brainstorm_skipped(state, event)?,

        "uncertainty_recorded" => cognitive::uncertainty_recorded(state, event)?,

        "evidence_recorded" => cognitive::evidence_recorded(state, event)?,

        "uncertainty_disposition_recorded" => {
            cognitive::uncertainty_disposition_recorded(state, event)?
        }

        "research_artifact_recorded" => cognitive::research_artifact_recorded(state, event)?,

        "subagent_dispatched" => cognitive::subagent_dispatched(state, event)?,
        _ => return Err(format!("Unknown event type: {}", event.event_type)),
    }

    state.last_seq = event.seq;
    state.processed_commands.insert(event.command_id.clone());
    state.history.push(event.event_id.clone());
    Ok(())
}

#[cfg(test)]
mod scope_tests {
    use super::{path_within_scope, scopes_overlap};

    #[test]
    fn within_scope_exact_and_descendant() {
        assert!(path_within_scope("src/auth", "src/auth"));
        assert!(path_within_scope("src/auth/token.rs", "src/auth"));
        assert!(path_within_scope("src/main.rs", "src"));
        assert!(path_within_scope("README.md", "README.md"));
    }

    #[test]
    fn within_scope_rejects_sibling_prefix_escape() {
        // The regression: a raw string prefix would wrongly admit these.
        assert!(!path_within_scope(
            "src/authentication/token.rs",
            "src/auth"
        ));
        assert!(!path_within_scope("src2/file.rs", "src"));
        assert!(!path_within_scope("README.md.bak", "README.md"));
        assert!(!path_within_scope("srcfoo", "src"));
    }

    #[test]
    fn within_scope_tolerates_trailing_slash_and_empty() {
        assert!(path_within_scope("src/auth/x.rs", "src/auth/"));
        assert!(!path_within_scope("anything", ""));
        assert!(!path_within_scope("anything", "/"));
    }

    #[test]
    fn overlap_is_segment_aware_and_symmetric() {
        // Disjoint siblings do NOT overlap (raw prefix would say they do).
        assert!(!scopes_overlap("src", "src2"));
        assert!(!scopes_overlap("src/auth", "src/authentication"));
        // Containment overlaps, both directions; equality overlaps.
        assert!(scopes_overlap("src", "src/auth"));
        assert!(scopes_overlap("src/auth", "src"));
        assert!(scopes_overlap("src/auth", "src/auth"));
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use crate::domain::event::Event;

    fn dispatch_event(task_id: &str, seq: i64, payload: serde_json::Value) -> Event {
        Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: format!("evt-{seq}"),
            command_id: format!("cmd-{seq}"),
            task_id: task_id.to_string(),
            seq,
            occurred_at: "2026-06-20T00:00:00Z".to_string(),
            actor: "human".to_string(),
            event_type: "subagent_dispatched".to_string(),
            payload,
        }
    }

    #[test]
    fn subagent_dispatched_records_host_attested_dispatch() {
        // subagent-dispatch-record-v1: the reducer records a dispatch onto the
        // parent task. role/adapter are host labels; the artifact is path+hash.
        let mut state = TaskState::new("t");
        let ev = dispatch_event(
            "t",
            1,
            serde_json::json!({
                "role": "designer",
                "adapter": "opencode",
                "parent_run": "run-1",
                "instruction_path": "src/x.md",
                "instruction_hash": "aaa",
                "trust_level": "content_l0"
            }),
        );
        apply(&mut state, &ev).unwrap();
        assert_eq!(state.dispatches.len(), 1);
        let d = &state.dispatches[0];
        assert_eq!(d.role, "designer");
        assert_eq!(d.adapter, "opencode");
        assert_eq!(d.parent_run.as_deref(), Some("run-1"));
        let instr = d.instruction.as_ref().expect("instruction recorded");
        assert_eq!(instr.path, "src/x.md");
        assert_eq!(instr.hash, "aaa");
        assert!(d.context.is_none() && d.output.is_none());
        // recorded_by is the envelope actor (an unattested label), never a payload field.
        assert_eq!(d.recorded_by, "human");
    }

    #[test]
    fn subagent_dispatched_requires_role_and_adapter() {
        let mut state = TaskState::new("t");
        let ev = dispatch_event(
            "t",
            1,
            serde_json::json!({"adapter": "opencode", "trust_level": "content_l0"}),
        );
        assert!(apply(&mut state, &ev).is_err(), "role is required");
    }

    #[test]
    fn subagent_dispatched_rejected_on_terminal_task() {
        // Terminal-is-terminal: a completed task's record must not change.
        let mut state = TaskState::new("t");
        state.phase = Phase::Completed;
        let ev = dispatch_event(
            "t",
            1,
            serde_json::json!({"role": "x", "adapter": "y", "trust_level": "content_l0"}),
        );
        assert!(apply(&mut state, &ev).is_err());
    }

    #[test]
    fn subagent_dispatched_rejects_half_artifact_pair() {
        // A path without its hash (or vice-versa) is rejected — no half pairs.
        let mut state = TaskState::new("t");
        let ev = dispatch_event(
            "t",
            1,
            serde_json::json!({
                "role": "x", "adapter": "y", "trust_level": "content_l0",
                "instruction_path": "src/x.md"
            }),
        );
        assert!(apply(&mut state, &ev).is_err());
    }
}

#[cfg(test)]
mod phase_guard_tests {
    use super::*;
    use crate::domain::event::Event;

    fn event(task_id: &str, seq: i64, event_type: &str, payload: serde_json::Value) -> Event {
        Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: format!("evt-{seq}"),
            command_id: format!("cmd-{seq}"),
            task_id: task_id.to_string(),
            seq,
            occurred_at: "2026-06-20T00:00:00Z".to_string(),
            actor: "human".to_string(),
            event_type: event_type.to_string(),
            payload,
        }
    }

    #[test]
    fn run_started_rejected_outside_in_progress() {
        // C10: a forged run_started bypassing ControlApp::run_start must not
        // leave a dangling active_run on a non-InProgress task.
        let mut state = TaskState::new("t");
        state.phase = Phase::Ready;
        let ev = event(
            "t",
            1,
            "run_started",
            serde_json::json!({"run_id": "r1", "adapter": "omp", "lease_id": "l1"}),
        );
        assert!(
            apply(&mut state, &ev).is_err(),
            "run_started must require InProgress"
        );
        assert!(
            state.active_run.is_none(),
            "no dangling active_run on rejection"
        );
    }

    #[test]
    fn run_started_accepted_in_progress() {
        let mut state = TaskState::new("t");
        state.phase = Phase::InProgress;
        let ev = event(
            "t",
            1,
            "run_started",
            serde_json::json!({"run_id": "r1", "adapter": "omp", "lease_id": "l1"}),
        );
        apply(&mut state, &ev).expect("accepted in InProgress");
        assert!(state.active_run.is_some());
    }

    #[test]
    fn hold_entered_rejected_on_completed_task() {
        // C11: holding a terminal task would make it permanently unarchivable
        // (task_archived is not hold-exempt).
        let mut state = TaskState::new("t");
        state.phase = Phase::Completed;
        let ev = event("t", 1, "hold_entered", serde_json::json!({}));
        assert!(apply(&mut state, &ev).is_err());
        assert!(!state.is_held, "is_held must not flip on rejection");
    }

    #[test]
    fn hold_entered_rejected_on_cancelled_task() {
        let mut state = TaskState::new("t");
        state.phase = Phase::Cancelled;
        let ev = event("t", 1, "hold_entered", serde_json::json!({}));
        assert!(apply(&mut state, &ev).is_err());
    }

    #[test]
    fn boundary_violation_rejected_on_terminal_task() {
        let mut state = TaskState::new("t");
        state.phase = Phase::Completed;
        let ev = event(
            "t",
            1,
            "boundary_violation_recorded",
            serde_json::json!({"path": "src/x.rs"}),
        );
        assert!(apply(&mut state, &ev).is_err());
        assert!(!state.is_held);
    }

    #[test]
    fn hold_entered_accepted_on_in_progress_task() {
        // Positive case: holding an active task is the normal flow.
        let mut state = TaskState::new("t");
        state.phase = Phase::InProgress;
        let ev = event("t", 1, "hold_entered", serde_json::json!({}));
        apply(&mut state, &ev).expect("accepted in InProgress");
        assert!(state.is_held);
    }
}
