use std::collections::HashMap;
use std::process::Command;

use anyhow::{bail, Result};
use rust_apt::cache::Upgrade;
use rust_apt::{new_cache, Cache, Marked, PkgCurrentState, Version};

use crate::history::Operation;
use crate::tui::summary::{SummaryPkg, SummaryTab};
use crate::util::sudo_check;
use crate::Config;

pub fn auto_remover(cache: &Cache) -> Vec<Version> {
	let mut marked_remove = vec![];
	for package in cache.iter() {
		if !package.is_auto_removable() {
			continue;
		}

		if package.current_state() != PkgCurrentState::ConfigFiles {
			package.mark_delete(false);
			if let Some(inst) = package.installed() {
				marked_remove.push(inst);
			}
		} else {
			package.mark_keep();
		}
	}
	// There is more code in private-install.cc DoAutomaticremove
	// If there are auto_remove bugs consider implementing that.
	marked_remove
}

pub fn upgrade(config: &Config) -> Result<()> {
	// sudo_check(config)?;
	let cache = new_cache!()?;

	cache.upgrade(Upgrade::FullUpgrade)?;

	let auto_remove = auto_remover(&cache);
	let mut pkg_set: HashMap<Operation, Vec<SummaryPkg>> = HashMap::new();

	for pkg in cache.get_changes(true) {
		let (op, ver) = match pkg.marked() {
			mark @ (Marked::NewInstall
			| Marked::Install
			| Marked::ReInstall
			| Marked::Downgrade) => {
				let Some(cand) = pkg.install_version() else {
					continue;
				};
				let op = match mark {
					Marked::ReInstall => Operation::Reinstall,
					Marked::Downgrade => Operation::Downgrade,
					_ => Operation::Install,
				};
				(op, cand)
			},
			Marked::Remove | Marked::Purge => {
				let Some(inst) = pkg.installed() else {
					continue;
				};

				if auto_remove.contains(&inst) {
					continue;
				}

				let op = if pkg.marked_purge() { Operation::Purge } else { Operation::Remove };
				(op, inst)
			},
			Marked::Upgrade => {
				if let (Some(inst), Some(cand)) = (pkg.installed(), pkg.candidate()) {
					pkg_set
						.entry(Operation::Upgrade)
						.or_default()
						.push(SummaryPkg::new(
							config,
							Operation::Upgrade,
							cand,
							Some(inst),
						));
				}
				continue;
			},
			// TODO: See if pkg is held for phasing and show percent
			// pkgDepCache::PhasingApplied
			// VerIterator::PhasedUpdatePercentage
			Marked::Held => {
				let Some(cand) = pkg.candidate() else {
					continue;
				};
				(Operation::Held, cand)
			},
			Marked::Keep => continue,
			Marked::None => bail!("{pkg} not marked, this should be impossible"),
		};

		pkg_set
			.entry(op)
			.or_default()
			.push(SummaryPkg::new(config, op, ver, None));
	}

	pkg_set.insert(
		Operation::AutoRemove,
		auto_remove
			.into_iter()
			.map(|v| SummaryPkg::new(config, Operation::AutoRemove, v, None))
			.collect(),
	);

	// create app and run it
	SummaryTab::new(&cache, config, pkg_set).run()?;

	let pre_invoke = config.apt.find_vector("DPkg::Pre-Invoke");
	config.apt.clear("DPkg::Pre-Invoke");

	run_scripts(pre_invoke)?;

	let post_invoke = config.apt.find_vector("DPkg::Post-Invoke");
	config.apt.clear("DPkg::Post-Invoke");

	run_scripts(post_invoke)
}

pub fn run_scripts(hooks: Vec<String>) -> Result<()> {
	for hook in hooks {
		println!("Running {hook}");
		let mut child = Command::new("sh")
			.arg("-c")
			.arg(hook)
			.spawn()?;

		let exit = child.wait()?;
		dbg!(exit);
		if !exit.success() {
			// TODO: Figure out how to return the ExitStatus from main.
			std::process::exit(exit.code().unwrap());
		}
	}
	Ok(())
}


pub fn apt_hook_with_pkgs() {

}
