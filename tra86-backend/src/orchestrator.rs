use crate::{BackendError, DebugBackend};

pub struct BackendOrchestrator {
    backend: Box<dyn DebugBackend>,
}

impl BackendOrchestrator {
    pub fn new(backend: Box<dyn DebugBackend>) -> Self {
        Self { backend }
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.backend_name()
    }

    pub fn backend_mut(&mut self) -> &mut dyn DebugBackend {
        self.backend.as_mut()
    }

    pub fn replace_backend(&mut self, backend: Box<dyn DebugBackend>) -> Result<(), BackendError> {
        self.backend = backend;
        Ok(())
    }
}
