from __future__ import annotations

import os
import sys
import subprocess
from pathlib import Path
from subprocess import run
from asyncio import run as aiorun
from typing import NoReturn
from httpx import get

from apt import Package
from nala import _, color
from nala.constants import ERROR_PREFIX
from nala.cache import Cache
from nala.dpkg import InstallProgress
from nala.utils import get_pkg_name
from nala.downloader import PkgDownloader
from nala.dpkg import InstallProgress


def get_all_deps(pkg: Package, pkg_set: set[Package]) -> None:
	"""Get all of the dependencies of a package recursively."""
	if not pkg.candidate:
		no_cand_error(pkg)

	for dep in pkg.candidate.dependencies:
		target = dep.target_versions[0].package
		# Target is in our set, so we've already recursed it.
		if target in pkg_set:
			continue
		# If not already in the set add it and recurse
		pkg_set.add(target)
		get_all_deps(target, pkg_set)

def chroot(command: list[str]) -> None:
	"""Run a command in the chroot."""
	cmd = ["fakechroot", "fakeroot", "chroot", "./boot-test/"]
	run(cmd + command, check=True)

def pkg_path(pkg: Package) -> str:
	"""Return the pkg path. Error if there is no candidate."""
	if not pkg.candidate:
		no_cand_error(pkg)
	return f"/var/cache/apt/archives/{get_pkg_name(pkg.candidate)}"

def no_cand_error(pkg: Package) -> NoReturn:
	"""Error and exit with no candidate."""
	sys.exit(
		_("{error} {package} has no candidate").format(
			error=ERROR_PREFIX, package=color(pkg.name, "YELLOW")
		)
	)

class Bootstrap:
	"""Class to organize the bootstrapping process."""

	def __init__(self, target: Path) -> None:
		"""Class to organize the bootstrapping process."""
		self.target = target
		self.cache = Cache()
		# Start with a base of only what is required
		self.required = [pkg for pkg in self.cache if pkg.candidate and pkg.candidate.priority == "required"]
		self.packages: set[Package] = set()
		# Core packages for the first install stage
		self.core: set[str] = {"base-passwd", "base-files", "dpkg", "libc6", "perl-base", "mawk", "debconf"}

		# This should really be handled better.
		# We don't need constants, they should be dynamic from the config,
		# but this is a lot of work
		self.archive = self.target / "var/cache/apt/archives/"
		self.partial = self.archive / "partial"

		# Make sure they exist before anything
		self.archive.mkdir(parents=True, exist_ok=True)
		self.partial.mkdir(parents=True, exist_ok=True)

		# self.task = dpkg_progress.add_task("", total=self.total_ops())
		# self.inst_progress = InstallProgress(bootstrap_log, bootstrap_log, live, task)

		# Add the required packages and their dependencies into all packages
		for pkg in self.required:
			# Make sure our packages make it in there
			self.packages.add(pkg)
			# Recurse the dependecies for each package
			get_all_deps(pkg, self.packages)

	def _target(self, path: Path | str) -> Path:
		"""Change a path into the target path."""
		# Have to remove the leading slash to append
		return self.target / f"{path}".lstrip("/")

	def dpkg(self, command: list[str], inst_progress: InstallProgress) -> None:
		"""Run a dpkg command in the chroot with Nala formatting."""
		cmd = ["fakechroot", "fakeroot", "chroot", f"{self.target}", "dpkg", "--force-depends"]
		process = subprocess.Popen(cmd + command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
		if not process.stdout:
			sys.exit(
				_("{error} error running {command}").format(
					error=ERROR_PREFIX, package=color(" ".join(cmd + command), "YELLOW")
				)
			)

		while True:
				output = process.stdout.readline()
				if output == b'' and process.poll() is not None:
					break
				if output.startswith((b"Unpacking", b"Setting up")):
					inst_progress.line_handler(output)
					inst_progress.advance_progress()
				else:
					inst_progress.term_log(output)

	def merge_usr(self) -> None:
		"""Symlink directories such as /bin => /usr/bin."""
		print("Merging /usr")
		self._target("/usr").mkdir()
		# Only AMD64 is supported atm
		links = ["bin", "sbin", "lib", "lib32", "lib64", "libx32"]
		for dir in links:
			self._target(dir).symlink_to(f"usr/{dir}", target_is_directory=True)
			self._target(f"usr/{dir}").mkdir()

	def download(self) -> None:
		"""Download the necessary packages to the target environment."""
		aiorun(PkgDownloader(self.packages, self.archive, self.partial).start_download())

	def extract_minimal(self) -> None:
		"""Extract the minimal system."""
		print("Extracting Minimal System")
		old_dir = os.getcwd()

		# Extract only the required packages
		files: list[Path] = []
		for pkg in self.required:
			if not pkg.candidate or pkg.candidate.priority != "required":
				continue
			file = (self.archive / get_pkg_name(pkg.candidate)).absolute()
			files.append(file)

		os.chdir(self.target)
		for file in files:
			# Do it like they do on the debootstrap channel
			# using dpkg-deb --extract does not respect merged usr
			tar = run(["dpkg-deb", "--fsys-tarfile", f"{file}"], capture_output=True).stdout
			# K is needed for usermerged
			run(["tar", "-xkf", "-"], input=tar)
		os.chdir(old_dir)

	def add_nala(self) -> None:
		"""Add extra packages to the main set."""
		# ca-certificates is needed for apt to access https mirrors
		for pkg_name in ("ca-certificates", "nala"):
			pkg = self.cache[pkg_name]
			self.packages.add(pkg)
			get_all_deps(pkg, self.packages)

	def install_core(self, inst_progress: InstallProgress) -> None:
		"""Install the core packages."""
		for pkg in self.packages:
			if pkg.name not in self.core:
				continue
			self.dpkg(["-i", pkg_path(pkg)], inst_progress)

		# Setup Etc files after core is installed
		# Maybe configure and ask the user?
		resolve = Path("/etc/resolv.conf")
		hostname = Path("/etc/hostname")

		self._target(resolve).write_bytes(resolve.read_bytes())
		self._target(hostname).write_bytes(hostname.read_bytes())
		# We might not need to do this for fakeroot?
		#target("/dev").symlink_to("/dev", target_is_directory=True)

	def install_and_configure(self, inst_progress: InstallProgress) -> None:
		"""Install and configure the system."""
		# Unpack the rest of the packages
		for pkg in self.packages:
			self.dpkg(["--unpack", pkg_path(pkg)], inst_progress)

		# Configure the system
		self.dpkg(["--configure", "--pending", "--force-configure-any"], inst_progress)

	def post_setup(self, install_nala: bool) -> None:
		"""Preform post installation setup. This may include setting up Nala."""
		# Write the sources.list file
		sources = self._target("etc/apt/sources.list")
		# TODO: Allow this to be configurable
		sources.write_text("deb http://deb.debian.org/debian/ sid main\n")

		if install_nala:
			# Setup Nala - Eventually this shouldn't have to be done when we get keyring packages.
			self._target("etc/apt/sources.list.d/volian-archive-scare-unstable.list").write_text("deb https://deb.volian.org/volian/ scar main")
			volian_key = get("https://deb.volian.org/volian/scar.key")
			volian_key.raise_for_status()
			self._target("etc/apt/trusted.gpg.d/volian-archive-scar-unstable.gpg").write_bytes(volian_key.content)

			chroot(["nala", "fetch", "--debian", "sid", "--country", "us", "--auto", "-y"])
			chroot(["nala", "install", "--update", "-y", "neofetch"])
		else:
			chroot(["apt-get", "update"])

	def total_ops(self) -> int:
		"""Total operations for dpkg to complete."""
		return (len(self.packages) + len(self.core)) * 2
