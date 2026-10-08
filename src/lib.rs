pub mod ast;
mod parser;

pub use parser::{ParseError, parse};

pub mod prelude {
    pub use crate::{ast::*, parse};
    pub use bevy::prelude::*;
    pub use tiny_bail::prelude::*;
}
