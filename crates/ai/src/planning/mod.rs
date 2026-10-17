//! Planning: the prompt that asks for a spec, user stories, and tasks, and the parser that checks the reply.
//!
//! The reply is untrusted text. The prompt asks for one JSON object, and the parser accepts only that object, with
//! every rule checked. Nothing in the reply beyond the schema is trusted. Assignees are matched against the team
//! here, so an invented name can't assign anyone.

mod parse;
mod prompt;

pub use parse::{
    ErrorKind, ParsedPlan, ParsedTask, PlanIssue, PlanParseError, PlanWarning, Problem, UserStory,
    parse_plan,
};
pub use parse::{RefineKind, Refined, Refinement, parse_refinement};
pub use prompt::{PROMPT_VERSION, build_prompt, build_refinement_prompt};
