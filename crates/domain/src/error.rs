use thiserror::Error;

/// A value broke a domain rule. Raised by constructors and mutators.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    #[error("a task needs a title")]
    EmptyTitle,
    #[error("a name can't be empty")]
    EmptyName,
    #[error("text can be at most {max} characters")]
    TooLong { max: usize },
    #[error("an estimate can't be negative (got {minutes} minutes)")]
    NegativeEstimate { minutes: i64 },
    #[error("an estimate must be a finite number")]
    NotFinite,
    #[error("an estimate is too large")]
    EstimateTooLarge,
    #[error("a position key can't be empty")]
    EmptyPosition,
    #[error("the '{status}' column is always shown and can't be hidden")]
    StatusNotHideable { status: String },
    #[error("'{0}' is not a valid status")]
    UnknownStatus(String),
    #[error("'{0}' is not a valid id")]
    InvalidId(String),
    #[error("the '{status}' column isn't shown on this project")]
    StatusNotVisible { status: String },
    #[error("the task is already in that column")]
    SameStatus,
    #[error("a workspace named '{name}' already exists")]
    DuplicateName { name: String },
    #[error("a project named '{name}' already exists in this workspace")]
    DuplicateProjectName { name: String },
    #[error("a description can be at most {max} characters")]
    DescriptionTooLong { max: usize },
    #[error("'{file_name}' is not a valid icon file name")]
    InvalidIconFile { file_name: String },
}

/// A repository could not load or save an entity.
#[derive(Debug, Error)]
pub enum RepoError {
    #[error("storage failed: {0}")]
    Storage(String),
    /// A uniqueness rule in storage rejected the write, for example a duplicate workspace name.
    #[error("that name is already taken")]
    NameTaken,
    /// Stored data exists but no longer satisfies the domain rules.
    #[error("stored data is invalid: {0}")]
    Invalid(#[from] DomainError),
}

pub type RepoResult<T> = Result<T, RepoError>;
