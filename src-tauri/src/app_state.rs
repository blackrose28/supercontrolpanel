use crate::stores::{
    BoundaryAutomationStore, FunctionStore, OpenVpnAdminStore, RemoteBrowserStore, SessionStore,
};

#[allow(dead_code)]
pub struct AppState {
    pub sessions: SessionStore,
    pub functions: FunctionStore,
    pub remote_browsers: RemoteBrowserStore,
    pub openvpn_admin: OpenVpnAdminStore,
    pub boundary_automation: BoundaryAutomationStore,
}
