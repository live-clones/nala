use anyhow::Result;

use rust_apt::new_cache;
use rust_apt::cache::Upgrade;

use crate::{util::sudo_check, Config};

pub fn upgrade(config: &Config) -> Result<()> {
	// sudo_check(config)?;
	let cache = new_cache!()?;


	cache.upgrade(Upgrade::FullUpgrade)?;

	for pkg in cache.get_changes(true) {
		if pkg.marked_delete() {
			let Some(inst) = pkg.installed() else {
				continue;
			};

			println!("'{inst}' will be REMOVED");
		}


		if let (Some(inst), Some(cand)) = (pkg.installed(), pkg.candidate()) {
			if pkg.marked_upgrade() {
				println!("'{pkg}' '{inst}' -> '{cand}'")
			}
		}
	}
	Ok(())
}
