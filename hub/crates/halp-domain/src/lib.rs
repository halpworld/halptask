//! HalpTask Hub domain model.
//!
//! Pure data and logic, no IO: items and the outline tree, typed tasks with JSON
//! Schema validated `fields`, natural-language due dates, quick capture, task
//! filters, layouts and roles, plus the input limits every entry point enforces.

pub mod capture;
pub mod dates;
pub mod error;
pub mod event;
pub mod filter;
pub mod grid;
pub mod integration;
pub mod item;
pub mod layout;
pub mod limits;
pub mod outline;
pub mod task_type;

pub use error::{DomainError, ErrorKind};
pub use event::{EntityRef, Event};
pub use filter::{StatusSet, TaskFilter};
pub use item::{Item, ItemKind, ItemView, Status, new_id};
pub use layout::{Layout, Role, Widget};
pub use outline::{ItemChange, Outline};
pub use task_type::{PackTrust, TaskType, TaskTypeDisplay, TaskTypeOrigin, TaskTypeRegistry};
