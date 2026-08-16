mod listener;
mod server;

pub use listener::{spawn_listener, ShutdownSender};

pub const TUNNEL_TOKEN_HEADER: &str = "x-mnelyra-tunnel-token";
pub const TUNNEL_SECRET_SCOPE: &str = "openai_connector";
pub const TUNNEL_MCP_SECRET_KEY: &str = "mcp_token";
