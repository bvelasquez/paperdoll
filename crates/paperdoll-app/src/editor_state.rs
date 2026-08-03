//! Editor lock shared with the HTTP thread: while the in-app editor is open, playback
//! triggers return 409 so authoring and agent commands do not fight.

use bevy::prelude::Resource;
use std::sync::{Arc, RwLock};

#[derive(Resource, Clone)]
pub struct SharedEditorState(pub Arc<RwLock<bool>>);

impl SharedEditorState {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(false)))
    }

    pub fn set_active(&self, active: bool) {
        *self.0.write().unwrap() = active;
    }

    pub fn is_active(&self) -> bool {
        *self.0.read().unwrap()
    }
}

impl Default for SharedEditorState {
    fn default() -> Self {
        Self::new()
    }
}
