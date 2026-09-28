//! Renderer preference owned by the runtime.
//!
//! The embedding client owns the host window and supplies its renderable
//! surface. Renderer policy stays here because the Android graphics APIs
//! exposed to Roblox, and the decision to offer Vulkan or GLES, are runtime
//! responsibilities.

/// Requested graphics backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackendPreference {
    /// Prefer Vulkan when the supplied surface and host loader support it;
    /// otherwise use OpenGL ES.
    #[default]
    Automatic,
    /// Require Vulkan.
    Vulkan,
    /// Require OpenGL ES.
    OpenGlEs,
}

/// Backend selected after checking host support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Vulkan,
    OpenGlEs,
}

impl BackendPreference {
    /// Resolve this preference against Vulkan support on the supplied surface.
    pub fn select(self, vulkan_available: bool) -> Result<Backend, BackendUnavailable> {
        match self {
            Self::Automatic if vulkan_available => Ok(Backend::Vulkan),
            Self::Automatic | Self::OpenGlEs => Ok(Backend::OpenGlEs),
            Self::Vulkan if vulkan_available => Ok(Backend::Vulkan),
            Self::Vulkan => Err(BackendUnavailable::Vulkan),
        }
    }
}

/// A requested backend cannot be supplied by the host surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendUnavailable {
    Vulkan,
}

impl std::fmt::Display for BackendUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vulkan => {
                f.write_str("Vulkan is required but unavailable for the supplied surface")
            }
        }
    }
}

impl std::error::Error for BackendUnavailable {}

#[cfg(test)]
mod tests {
    use super::{Backend, BackendPreference};

    #[test]
    fn automatic_prefers_vulkan_and_falls_back_to_gles() {
        assert_eq!(
            BackendPreference::Automatic.select(true),
            Ok(Backend::Vulkan)
        );
        assert_eq!(
            BackendPreference::Automatic.select(false),
            Ok(Backend::OpenGlEs)
        );
    }

    #[test]
    fn explicit_backend_is_respected() {
        assert_eq!(
            BackendPreference::OpenGlEs.select(true),
            Ok(Backend::OpenGlEs)
        );
        assert_eq!(BackendPreference::Vulkan.select(true), Ok(Backend::Vulkan));
        assert!(BackendPreference::Vulkan.select(false).is_err());
    }
}
