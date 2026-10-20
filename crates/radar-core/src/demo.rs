//! A host over parts with their sample data, for tests and the sample screens:
//! no config, the clock at [`DEMO_NOW`].

use anyhow::Result;

use crate::host::Host;
use crate::plugin::{DEMO_NOW, Plugin};

/// A host over these parts with no config, their sample data, the clock at
/// [`DEMO_NOW`].
pub fn demo_host(plugins: Vec<Box<dyn Plugin>>) -> Result<Host> {
    let mut host = Host::new(plugins, DEMO_NOW)?;
    host.load_config(Some(""))?;
    // Loading a config reconfigures the parts: their samples go in after.
    for p in &mut host.plugins {
        p.demo();
    }
    Ok(host)
}
