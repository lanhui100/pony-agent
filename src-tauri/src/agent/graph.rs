pub struct GraphEngine {
    name: &'static str,
}

impl GraphEngine {
    pub fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub fn name(&self) -> &str {
        self.name
    }
}
