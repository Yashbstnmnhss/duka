use std::collections::HashMap;

use duka_shared::types::{LogicDatabase, Term};

pub struct Unifier {
    bindings: HashMap<String, Term>,
}

impl Unifier {}

pub struct Inference {
    database: LogicDatabase,
    unifier: Unifier,
}
