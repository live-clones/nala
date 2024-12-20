use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};
use rust_apt::{Cache, Marked, Package, PkgCurrentState, Version};

use crate::cmd::{HistoryPackage, Operation};
use crate::config::color;
use crate::{debug, info, warn};

type SortedChanges = HashMap<Operation, Vec<HistoryPackage>>;

pub trait NalaCache {
	fn sort_changes(&self) -> Result<SortedChanges>;
	fn auto_remove(&self) -> Vec<Version>;
}

pub trait NalaPkg<'a> {
	fn filter_virtual(self) -> Result<Package<'a>>;
}

impl<'a> NalaPkg<'a> for Package<'a> {
	fn filter_virtual(self) -> Result<Package<'a>> {
		if self.has_versions() {
			return Ok(self);
		}

		// Package is virtual so get its providers.
		// HashSet for duplicated packages when there is more than one version
		// clippy thinks that the package is mutable
		// But it only hashes the ID and you can't really mutate a package
		#[allow(clippy::mutable_key_type)]
		let providers: HashSet<Package> = self.provides().map(|p| p.package()).collect();

		// If the package doesn't have provides it's purely virtual
		// There is nothing that can satisfy it. Referenced only by name
		// At time of commit `python3-libmapper` is purely virtual
		if providers.is_empty() {
			warn!(
				"{} has no providers and is purely virutal",
				color::primary!(self.name())
			);

			return Ok(self);
		}

		// If there is only one provider just select that as the target
		if providers.len() == 1 {
			// Unwrap should be fine here, we know that there is 1 in the Vector.
			let target = providers.into_iter().next().unwrap();
			info!(
				"Selecting {} instead of virtual package {}",
				color::primary!(target.fullname(false)),
				color::primary!(self.name())
			);
			return Ok(target);
		}

		// If there are multiple providers then we will error out
		// and show the packages the user could select instead.
		info!(
			"{} is a virtual package provided by:",
			color::primary!(self.name())
		);

		for target in &providers {
			// If the version doesn't have a candidate no sense in showing it
			if let Some(cand) = target.candidate() {
				println!(
					"    {} {}",
					color::primary!(target.fullname(true)),
					color::ver!(cand.version()),
				);
			}
		}
		bail!("You should select just one.")
	}
}

impl NalaCache for Cache {
	/// Run the autoremover and then get the changes from the cache.
	fn sort_changes(&self) -> Result<SortedChanges> {
		debug!("Running auto_remover");

		let auto_remove = self.auto_remove();
		let mut pkg_set: HashMap<Operation, Vec<HistoryPackage>> = HashMap::new();

		debug!("Calculating changes");
		let changed = self.get_changes(true).collect::<Vec<_>>();
		if changed.is_empty() {
			return Ok(pkg_set);
		}

		for pkg in &changed {
			let (op, ver) = match pkg.marked() {
				mark @ (Marked::NewInstall | Marked::Install | Marked::ReInstall) => {
					let Some(cand) = pkg.install_version() else {
						continue;
					};
					let op = match mark {
						Marked::ReInstall => Operation::Reinstall,
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
				mark @ (Marked::Upgrade | Marked::Downgrade) => {
					if let (Some(inst), Some(cand)) = (pkg.installed(), pkg.candidate()) {
						let op = match mark {
							Marked::Upgrade => Operation::Upgrade,
							_ => Operation::Downgrade,
						};

						pkg_set
							.entry(op)
							.or_default()
							.push(HistoryPackage::from_version(op, &cand, &Some(inst)));
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
				.push(HistoryPackage::from_version(op, &ver, &None));
		}

		if !auto_remove.is_empty() {
			pkg_set.insert(
				Operation::AutoRemove,
				auto_remove
					.into_iter()
					.map(|v| HistoryPackage::from_version(Operation::AutoRemove, &v, &None))
					.collect(),
			);
		}

		Ok(pkg_set)
	}

	fn auto_remove(&self) -> Vec<Version> {
		let mut marked_remove = vec![];
		for package in self.iter() {
			if !package.is_installed() {
				continue;
			}
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
}
