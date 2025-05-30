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
# nala is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# nala is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with nala.  If not, see <https://www.gnu.org/licenses/>.
"""Nala package manager."""

from __future__ import annotations

import os
import sys
from subprocess import run

# pylint: disable=subprocess-run-check

# Stop this code from running during shell completions
if not os.environ.get("_NALA_COMPLETE"):
	# If nala is ran with no arguments so the man page
	if len(sys.argv) == 1:
		sys.exit(run(["man", "nala"]).returncode)


# Set Path as below for termux environment
# ROOT = "@TERMUX_PREFIX@"
# USR = "@TERMUX_PREFIX@"
# TERMUX = True
ROOT = ""
USR = "/usr"
TERMUX = False

# pylint: disable=wrong-import-position
# Before we import anything else we need to make sure that this is in our path
# Else apt_pkg will give an import error. Conda is an example of this breaking.
# Additionally if something is installed with pip and pulls in dependencies
# That Nala uses this can also break. This fix is to add dist-packages to the beginning
# So it's always queried first.
sys.path.insert(0, f"{USR}/lib/python3/dist-packages")

import gettext

import apt_pkg
from rich.console import Console

# Debian needs to be initialized before the apt config.
# Inside debian.debian_support they init apt_pkg without checking if it's been initialized.
# If this is imported after apt_pkg.config is setup,
# it causes duplicates for things like pre/post invoke hooks
# https://gitlab.com/volian/nala/-/issues/287
import debian.debian_support  # isort:skip

if "APT" not in apt_pkg.config:
	apt_pkg.init_config()
apt_pkg.init_system()

__version__ = "0.16.0"

console = Console()

COLOR_CODES: dict[str, str | int] = {
	"RESET": "\x1b[0m",
	"ITALIC": "\x1b[3m",
	"RED": 31,
	"GREEN": 32,
	#'YELLOW' : 33,
	# We set Yellow as more of an orange for light terminals.
	"YELLOW": "38;5;202",
	"BLUE": 34,
	"MAGENTA": 35,
	"CYAN": 36,
	"WHITE": 37,
}


def color(text: object, text_color: str = "") -> str:
	"""Return colored text if allowed."""
	if os.environ.get("FORCE_COLOR"):
		return color_text(text, text_color)
	if (
		os.environ.get("NO_COLOR")
		or not console.is_terminal
		or console.is_dumb_terminal
	):
		return f"{text}"
	return color_text(text, text_color)


def color_text(text: object, text_color: str = "") -> str:
	"""Return bold text in the color of your choice."""
	return (
		f"\x1b[1;{COLOR_CODES[text_color]}m{text}{COLOR_CODES['RESET']}"
		if text_color
		else f"\x1b[1m{text}{COLOR_CODES['RESET']}"
	)


def color_version(version: str) -> str:
	"""Color version number."""
	if not console.is_terminal or console.is_dumb_terminal:
		return f"({version})"
	return f"{color('(')}{color(version, 'BLUE')}{color(')')}"


translate = gettext.translation("nala", fallback=True)


def _(msg: str) -> str:
	"""Gettext translator."""
	return msg if console.options.ascii_only else translate.gettext(msg)
