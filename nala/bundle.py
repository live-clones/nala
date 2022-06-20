#                 __
#    ____ _____  |  | _____
#   /    \\__  \ |  | \__  \
#  |   |  \/ __ \|  |__/ __ \_
#  |___|  (____  /____(____  /
#       \/     \/          \/
#
# Copyright (C) 2021, 2022 Blake Lee
#
# This file is part of nala
#
# nala is program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# nala is program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with nala.  If not, see <https://www.gnu.org/licenses/>.
"""The Nala bundle module."""
from __future__ import annotations

import gzip
import json
import sys
from base64 import b64encode
from dataclasses import dataclass
from pathlib import Path
from subprocess import run
from typing import Generator, cast
import apt_pkg

import jsbeautifier
import typer
from apt.package import Package, Version
from apt_pkg import MetaIndex

from nala import _, color
from nala.cache import Cache
from nala.constants import (
	APT_CONF,
	APT_CONF_PARTS,
	ERROR_PREFIX,
	LISTS_DIR,
	NALA_BUNDLE,
	NALA_CONF,
	PREFERENCES,
	PREFERENCES_PARTS,
	SOURCELIST,
	SOURCEPARTS,
	TRUSTED,
	TRUSTEDPARTS,
)
from nala.history import get_history, get_list
from nala.options import DEBUG, MAN_HELP, VERBOSE, arguments, nala
from nala.utils import DelayedKeyboardInterrupt, dprint, eprint


@dataclass(frozen=True, eq=True)
class SourceFile:
	"""Represent a Source File."""

	meta_index: MetaIndex
	source_list: Path
	pkg_file: Path
	gpg_key: Path
	contents: str

	def __str__(self) -> str:
		"""Return string representation of the source file."""
		return (
			"SourceFile = [\n"
			f"    Source List = {self.source_list},\n"
			f"    Package File = {self.pkg_file},\n"
			f"    GPG Key = {self.gpg_key},\n"
			"]"
		)


class Repos:
	"""Manage the Source Repos."""

	def __init__(self, cache: Cache) -> None:
		"""Manage the Source Repos."""
		self.cache = cache
		self.source_files: set[SourceFile] = set()
		self.describe: dict[str, SourceFile] = {}

	def make_describe(self) -> None:
		"""Generate hashmaps of descriptions to verify against."""
		if self.describe:
			return
		for source_file in self.source_files:
			for index in source_file.meta_index.index_files:
				self.describe[index.describe] = source_file

	def is_external_pkg(self, version: Version) -> SourceFile | None:
		"""Check if a packages is external or not."""
		self.make_describe()
		for pkg_file, _not_used in version._cand.file_list:
			indexfile = self.cache._list.find_index(pkg_file)
			if indexfile and indexfile.describe in self.describe:
				return self.describe[indexfile.describe]
		return None

	def build_sources(self) -> None:
		"""Build the sources cache."""
		temp_files: set[Path] = set()
		release_files = tuple(self.release_files())
		sources = {
			# Key = Path("/etc/apt/sources.list.d/volian-archive-scar-unstable.list")
			# Value = String Contents of the file
			file: file.read_text(encoding="utf-8", errors="replace")
			for file in (*SOURCEPARTS.iterdir(), SOURCELIST)
			if not file.name.endswith(".save")
		}

		for keyring in self.get_keys(sources):
			dprint(f"Starting Key: {keyring}")
			if unarmored := self.unarmor_ascii(keyring):
				temp_files.add(unarmored)

			for file in release_files:
				dprint(f"KeyRing: {color(keyring, 'BLUE')},\nFile: {file}")
				gpg_cmd: list[str | Path] = [
					"gpg",
					"--no-default-keyring",
					"--keyring",
					unarmored or keyring,
					"--verify",
					file,
				]
				# pylint: disable=subprocess-run-check
				if run(gpg_cmd, capture_output=True).returncode:
					continue
				dprint(f"{keyring} {color('verified', 'GREEN')}")
				for meta_index in self.cache._list.list:
					# deb.volian.org_volian_dists_scar_InRelease
					domain = file.name.split("_")[0]
					# meta_index = http://deb.volian.org/volian/
					if domain not in meta_index.uri:
						continue

					for path, contents in sources.items():
						# Take off the trailing slash.
						# Could probably leverage Regex in here quite a bit.
						if meta_index.uri.rstrip("/") not in contents:
							continue

						self.source_files.add(
							SourceFile(
								meta_index=meta_index,
								source_list=path,
								pkg_file=file,
								gpg_key=keyring,
								contents=contents,
							)
						)
		self.clean_up_files(temp_files)

	@staticmethod
	def release_files() -> Generator[Path, None, None]:
		"""Dedupe InRelease files."""
		release_files = tuple(
			file for file in LISTS_DIR.iterdir() if "InRelease" in file.name
		)
		dedup: set[bytes] = set()
		for file in release_files:
			if (data := file.read_bytes()) in dedup:
				continue
			dedup.add(data)
			yield file

	@staticmethod
	def get_keys(
		sources: dict[Path, str], all_keys: bool = False
	) -> Generator[Path, None, None]:
		"""Generate gpg keys."""
		yield from TRUSTEDPARTS.iterdir()
		# Find extra keys that could be defined within the sources.list
		for contents in sources.values():
			for line in contents.splitlines():
				# Deb822 Signed-By:
				if line.startswith("Signed-By:"):
					yield Path(line.split()[1])
				# deb [arch=amd64 signed-by=/usr/share/keyrings/deriv-archive-keyring.gpg]
				# http://deb.volian.org/volian/ scar main
				elif "signed-by=" in line:
					for option in line[line.find("[") + 1 : line.find("]")].split():
						if "signed-by=" in option:
							yield Path(option.split("=")[1])

		yield from (
			file
			for file in Path("/usr/share/keyrings/").iterdir()
			if (all_keys or not file.name.startswith(("debian", "ubuntu", "pop")))
		)

	@staticmethod
	def unarmor_ascii(keyring: Path) -> None | Path:
		"""Dearmor asc key."""
		with open(keyring, "rb") as file:
			if not file.readline().startswith(b"-----BEGIN PGP"):
				return None
			if (unarmored := Path(f"/tmp/{keyring.name}.gpg")).exists():
				unarmored.unlink()
			# pylint: disable=subprocess-run-check
			if run(("gpg", "--dearmor", "--output", unarmored, keyring)).returncode:
				return None
			dprint(f"Filename: {keyring} has been dearmored.")
			return unarmored

	@staticmethod
	def clean_up_files(temp_files: set[Path]) -> None:
		"""Clean up temporary files."""
		for path in temp_files:
			path.unlink()

	def print_keys(self) -> None:
		"""Print the keys. Can't imagine this sticks around."""
		print("\nVerified:")
		for source_file in self.source_files:
			print(source_file)


def pkg_gen(
	cache: Cache,
	manually_installed: bool,
	all_pkgs: bool,
	apt_pkgs: bool,
) -> Generator[Package, None, None]:
	"""Generate packages.

		The default with none of these arguments
		is Only pkgs explicitly installed with nala


		manual: Choose only pkgs manually installed, not essential

		all_pkgs: Choose all installed pkgs
	"""
	if all_pkgs:
		yield from (pkg for pkg in cache if pkg.is_installed)

	if apt_pkgs:
		yield from (pkg for pkg in cache if pkg.is_installed and not pkg.is_auto_installed)

	elif manually_installed:
		yield from (
			pkg
			for pkg in cache
			if pkg.is_installed
			and (version := pkg.candidate or pkg.installed)
			and not pkg.is_auto_installed
			and not pkg.essential
			and version.priority
			not in ("essential", "required", "standard", "important")
		)
	else:
		yield from (
			cache[name]
			for name in get_list(get_history("Nala"), "User-Installed")
			if name in cache
		)

def write_bundle_file(data: dict[str, dict[str, str] | list[str]]) -> None:
	"""Write history to file."""
	with DelayedKeyboardInterrupt():
		with open(NALA_BUNDLE, "wb") as file:
			file.write(
				jsbeautifier.beautify(
					json.dumps(data),
					jsbeautifier.BeautifierOptions(options={"indent_with_tabs": True}),
				).encode("utf-8")
				# json.dumps(data, separators=(",", ":")).encode("utf-8")
				# gzip.compress(json.dumps(data, separators=(",", ":")).encode("utf-8"))
			)


def encode_file(path: Path) -> str:
	"""Read file and return the contents as text or base64."""
	return b64encode(path.read_bytes()).decode(encoding="utf-8")


def recurse_paths(*paths: Path) -> Generator[Path, None, None]:
	"""Recurse paths and yield each file."""
	for path in paths:
		if not path.exists():
			continue
		if path.is_file():
			yield path
		if path.is_dir():
			yield from recurse_paths(*path.iterdir())


def dump_sources(sources: set[SourceFile]) -> dict[str, str]:
	"""Dump source files with gpg key."""
	return {
		f"{path}": encode_file(path)
		for source in sources
		for path in (source.source_list, source.gpg_key)
	}


OPTION_MAP = {
	"apt": APT_CONF,
	"apt.d": APT_CONF_PARTS,
	"nala": NALA_CONF,
	"pref": PREFERENCES,
	"pref.d": PREFERENCES_PARTS,
}


def dump_paths(options: set[str]) -> dict[str, str]:
	"""Dump preference files."""
	return {
		f"{path}": encode_file(path)
		for path in recurse_paths(*(OPTION_MAP[opt] for opt in options))
	}


def dump_hooks() -> dict[str, str]:
	"""Dump hook files."""
	return {
		f"{path}": encode_file(path)
		for path in (
			# If we have a dict we only need to grab the hook path to backup
			Path(cast(str, hook.get("hook", "")))
			if isinstance(hook, dict)
			else Path(hook)
			# Get all the hook types
			for hook_type in ("PreInstall", "PostInstall")
			for hook in arguments.config.get_hook(hook_type).values()
		)
		# If the path doesn't exist then the hook is a command
		if path.exists()
	}

TAB = "\t"
APT_LIST_COLORED = f"{color('apt list', 'GREEN')} {color('--manually-installed', 'BLUE')}"

def validate_options(options: set[str]) -> None:
	"""Check validity of the options."""
	if invalid := tuple(opt for opt in options if opt not in OPTION_MAP and opt != ""):
		eprint(
			_("{error} The following options are invalid.").format(error=ERROR_PREFIX)
		)
		sys.exit(f"  {', '.join(color(opt, 'YELLOW') for opt in invalid)}")


@nala.command(short_help = _("Backup files to be restored with Nala."))
# pylint: disable=unused-argument
def backup(
	nala_conf: bool = typer.Option(
		True,
		"--nala / --no-nala",
		show_default=False,
	),
	apt_conf: bool = typer.Option(
		True,
		"--apt / --no-apt",
		show_default=False,
	),
	apt_conf_dir: bool = typer.Option(
		False,
		"--apt-dir / --no-apt-dir",
		show_default=False,
	),
	pref: bool = typer.Option(
		True,
		"--pref / --no-pref",
		show_default=False,
	),
	pref_dir: bool = typer.Option(
		False,
		"--pref-dir / --no-pref-dir",
		show_default=False,
	),
	apt_installed: bool = typer.Option(
		False,
		"--apt-installed",
		show_default=False,
	),

	manually_installed: bool = typer.Option(
		False,
		"--manually-installed",
		show_default=False,
	),
	all_pkgs: bool = typer.Option(
		False,
		"--all-installed",
		show_default=False,
	),
	debug: bool = DEBUG,
	verbose: bool = VERBOSE,
	man_help: bool = MAN_HELP,
) -> None:
	"""Nala bundle command."""
	print(
		len(
			list(
				pkg_gen(cache := Cache(), manually_installed, all_pkgs, apt_installed)
			)
		)
	)

	# validate_options(conf_options | pref_options)

	# repos = Repos(cache := Cache())
	# repos.build_sources()

	# unique_sources: set[SourceFile] = set()
	# packages: list[str] = []
	# for pkg in pkg_gen(cache, pkg_type=pkg_type):
	# 	packages.append(pkg.name)
	# 	if not (version := pkg.candidate):
	# 		print(f"{pkg.name} has no candidate")
	# 		continue

	# 	if source_file := repos.is_external_pkg(version):
	# 		unique_sources.add(source_file)
	# 		print(f"\n{pkg.name} is an external package originating from")
	# 		print(source_file)
	# print(f"Total Sources for External Packages = {len(unique_sources)}")

	# nala_bundle: dict[str, dict[str, str] | list[str]] = {}
	# nala_bundle["Sources"] = dump_sources(unique_sources)
	# nala_bundle["Conf"] = dump_paths(conf_options)
	# nala_bundle["Hooks"] = dump_hooks()
	# nala_bundle["Preferences"] = dump_paths(pref_options)
	# nala_bundle["Packages"] = packages

	# write_bundle_file(nala_bundle)


# @bundle_typer.command("dump", help=_("Clear a transaction or the entire history."))
# # pylint: disable=unused-argument
# def dump(
# 	debug: bool = DEBUG,
# 	verbose: bool = VERBOSE,
# ) -> None:
# 	"""Nala dump command."""
