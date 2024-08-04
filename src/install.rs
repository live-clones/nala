use anyhow::Result;
use rust_apt::progress::{AcquireProgress, InstallProgress};
use rust_apt::{new_cache, Cache};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::config::Config;
use crate::dpkg;
use crate::util::sudo_check;

/// The function just runs apt's update and is designed to go into
/// it's own little thread.
pub fn install_thread() -> Result<()> {
	let cache = new_cache!()?;

	let pkg = cache.get("neofetch").unwrap();

	if pkg.is_installed() {
		pkg.mark_delete(true);
	} else {
		pkg.mark_install(true, true);
	}

	cache.resolve(false)?;

	dpkg::run_install(cache)?;

	Ok(())
}

#[tokio::main]
pub async fn install(config: &Config) -> Result<()> {
	sudo_check(config)?;

	let (tx, mut rx): (
		UnboundedSender<dpkg::Message>,
		UnboundedReceiver<dpkg::Message>,
	) = mpsc::unbounded_channel();

	config.apt.set("Dpkg::Use-Pty", "0");
	// config.apt.set("Debug::APT::Progress::PackageManagerFd", "1");
	let cache = new_cache!()?;

	let pkg = cache.get("neofetch").unwrap();

	if pkg.is_installed() {
		pkg.mark_delete(true);
	} else {
		pkg.mark_install(true, true);
	}

	cache.resolve(false)?;

	dpkg::run_install(cache)?;

	// install_thread()?;

	// let task = tokio::task::spawn(install_thread(inst_progress));

	// while let Some(msg) = rx.recv().await {
	// 	match msg {
	// 		dpkg::Message::Yes(msg) => {
	// 			println!("\n\n{msg}\n\n")
	// 		},
	// 		dpkg::Message::No(_) => todo!(),
	// 	}
	// }

	// task.await??;

	Ok(())
}
