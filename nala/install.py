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
"""Functions for the Nala Install command."""
from __future__ import annotations

import apt_pkg
from apt import Cache, Package, Version

from nala.show import print_dep
from nala.utils import color, dprint, term


def install_main(pkg_names: list[str], cache: Cache) -> bool:
	"""Mark pkg as install or upgrade."""
	with cache.actiongroup(): # type: ignore[attr-defined]
		for pkg_name in pkg_names:
			if pkg_name in cache:
				pkg = cache[pkg_name]
				try:
					if not pkg.installed:
						pkg.mark_install()
						dprint(f"Marked Install: {pkg.name}")
					elif pkg.is_upgradable:
						pkg.mark_upgrade()
						dprint(f"Marked upgrade: {pkg.name}")
				except apt_pkg.Error as error:
					if ('broken packages' not in str(error)
					and 'held packages' not in str(error)):
						raise error
					return False
	return True

def check_broken(pkg_names: list[str], cache: Cache) -> tuple[list[Package], list[str]]:
	"""Check if packages will be broken."""
	broken_count = 0
	not_found: list[str] = []
	broken: list[Package] = []
	depcache = cache._depcache

	with cache.actiongroup(): # type: ignore[attr-defined]
		for pkg_name in pkg_names:
			if pkg_name not in cache:
				not_found.append(pkg_name)
			else:
				pkg = cache[pkg_name]
				if pkg.installed and not pkg.is_upgradable:
					print(
						f"Package {color(pkg.name, 'GREEN')}",
						'is already at the latest version',
						color(pkg.installed.version, 'BLUE')
						)
					continue
				depcache.mark_install(pkg._pkg, False, True)
				if depcache.broken_count > broken_count:
					broken.append(pkg)
					broken_count += 1
	return broken, not_found

def print_broken(pkg_name: str, candidate: Version) -> None:
	"""Print broken packages information."""
	print('='*term.columns)
	version = color('(') + color(candidate.version, 'BLUE') + color(')')
	print(f"{color('Package:', 'YELLOW')} {color(pkg_name, 'GREEN')} {version}")
	if conflicts := candidate.get_dependencies('Conflicts'):
		print_dep(color('Conflicts:', 'YELLOW'), conflicts)
	if breaks := candidate.get_dependencies('Breaks'):
		print_dep(color('Breaks:', 'YELLOW'), breaks)
	if candidate.dependencies:
		print_dep(color('Depends:', 'YELLOW'), candidate.dependencies)