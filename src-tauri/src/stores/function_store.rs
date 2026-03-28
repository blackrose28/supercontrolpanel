use std::{collections::HashMap, sync::Mutex};

use crate::models::FunctionStatus;

pub struct FunctionStore {
    pub statuses: Mutex<HashMap<String, FunctionStatus>>,
}
