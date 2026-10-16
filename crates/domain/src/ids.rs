//! Typed identifiers. Each entity has its own ID type, so a `TaskId` can't be passed where a `ProjectId` is expected.

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(uuid::Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4())
            }

            pub fn from_uuid(uuid: uuid::Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(&self) -> uuid::Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

id_type!(WorkspaceId);
id_type!(ProjectId);
id_type!(TaskId);
id_type!(ColumnId);
id_type!(TeamMemberId);
id_type!(ActivityId);
id_type!(PlanDraftId);
