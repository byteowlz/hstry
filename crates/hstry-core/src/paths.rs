use std::path::PathBuf;

pub fn state_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".local").join("state")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hstry")
}

/// Path to the service port file (for TCP transport).
pub fn service_port_path() -> PathBuf {
    state_dir().join("service.port")
}

/// Path to the Unix domain socket (for Unix transport).
pub fn service_socket_path() -> PathBuf {
    state_dir().join("service.sock")
}
