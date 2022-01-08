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
"""Functions for handling Nala History."""
from __future__ import annotations

import sys
import json
from json.decoder import JSONDecodeError
from datetime import datetime
from shutil import get_terminal_size
from typing import TYPE_CHECKING

import jsbeautifier

from nala.logger import dprint
from nala.utils import print_packages
from nala.constants import COLUMNS, ERROR_PREFIX, NALA_HISTORY
from nala.rich import Table, Column, console

if TYPE_CHECKING:
	from nala.nala import Nala

def load_history_file() -> dict[str, dict[str, str | list[str] | list[list[str]]]]:
	"""Loads Nala history"""
	try:
		check = json.loads(NALA_HISTORY.read_text(encoding='utf-8'))
		assert isinstance(check, dict)
		return check
	except JSONDecodeError:
		sys.exit(ERROR_PREFIX+f"History file seems corrupt. You should try removing {NALA_HISTORY}")

def write_history_file(data: dict[str, dict[str, str | list[str] | list[list[str]]]]) -> None:
	"""Write history to file."""
	options = jsbeautifier.default_options()
	options.indent_size = 1
	options.indent_char = '\t'
	with open(NALA_HISTORY, 'w', encoding='utf-8') as file:
		file.write(jsbeautifier.beautify(json.dumps(data), options))

def history() -> None:
	"""Method for the history command."""
	if not NALA_HISTORY.exists():
		print("No history exists..")
		return
	history_file = load_history_file()
	names: list[tuple[str, str, str, str]] = []

	for key, entry in history_file.items():
		command = get_hist_list(entry, 'Command')
		if command[0] in ('update', 'upgrade'):
			for package in get_hist_package(entry, 'Upgraded'):
				command.append(package[0])
		names.append(
				(key, ' '.join(command), str(entry.get('Date')), str(entry.get('Altered')))
		)

	max_width = get_terminal_size().columns - 50
	history_table = Table(
				'ID',
				Column('Command', no_wrap=True, max_width=max_width),
				'Date and Time',
				'Altered',
				padding=(0,2), box=None
			)

	for item in names:
		history_table.add_row(*item)
	console.print(history_table)

def get_hist_package(hist_entry: dict[str, str | list[str] | list[list[str]]], key: str) -> list[list[str]]:
	"""Type enforce history package is list of lists."""
	return [pkg_list for pkg_list in hist_entry[key] if isinstance(pkg_list, list)]

def get_hist_list(hist_entry: dict[str, str | list[str] | list[list[str]]], key: str) -> list[str]:
	"""Type enforce history package is list of strings."""
	return [pkg for pkg in hist_entry[key] if isinstance(pkg, str)]

def history_info(hist_id: str) -> None:
	"""Method for the history info command."""
	dprint(f"History info {hist_id}")
	hist_entry = get_history(hist_id)
	dprint(f"History Entry: {hist_entry}")

	delete_names = get_hist_package(hist_entry, 'Removed')
	install_names = get_hist_package(hist_entry, 'Installed')
	upgrade_names = get_hist_package(hist_entry, 'Upgraded')

	print_packages(
		['Package:', 'Version:', 'Size:'],
		delete_names, 'Removed:', 'bold red')
	print_packages(
		['Package:', 'Version:', 'Size:'],
		install_names, 'Installed:', 'bold green')
	print_packages(
		['Package:', 'Old Version:', 'New Version:', 'Size:'],
		upgrade_names, 'Upgraded:', 'bold blue'
	)

	print('='*COLUMNS)
	if delete_names:
		print(f'Removed {len(delete_names)} Packages')
	if install_names:
		print(f'Installed {len(install_names)} Packages')
	if upgrade_names:
		print(f'Upgraded {len(upgrade_names)} Packages')

def history_clear(hist_id: str) -> None:
	"""Method for the show command."""
	dprint(f"History clear {hist_id}")
	if not NALA_HISTORY.exists():
		print("No history exists to clear..")
		return

	if hist_id == 'all':
		NALA_HISTORY.unlink()
		print("History has been cleared")
		return

	history_file = load_history_file()
	history_edit: dict[str, dict[str, str | list[str] | list[list[str]]]] = {}
	num = 0
	# Using sum increments to relabled the IDs so when you remove just one
	# There isn't a gap in ID numbers and it looks concurrent.
	for key, value in history_file.items():
		if key != hist_id:
			num += 1
			history_edit[str(num)] = value
	print('History has been altered...')
	write_history_file(history_edit)

def history_undo(apt: Nala, hist_id: str, redo: bool = False) -> None:
	"""History undo/redo commands."""
	if redo:
		dprint(f"History: redo {hist_id}")
	else:
		dprint(f"History: undo {hist_id}")
	transaction = get_history(hist_id)
	dprint(f"Transaction: {transaction}")

	command = transaction.get('Command', [''])[0]
	# We just reverse whatever was done in the transaction
	def hist_pkg_sort(key: str) -> list[str]:
		packages = transaction.get(key)
		if packages is None:
			sys.exit(ERROR_PREFIX+f"Something is wrong with history entry: {hist_id}")
		return [str(pkg[0]) for pkg in packages]

	if command == 'remove':
		pkgs = hist_pkg_sort('Removed')
		if redo:
			apt.remove(pkgs)
		else:
			apt.install(pkgs)

	elif command == 'install':
		pkgs = hist_pkg_sort('Installed')
		if redo:
			apt.install(pkgs)
		else:
			apt.remove(pkgs)
	else:
		print('Undo for operations other than install or remove are not currently supported')

def write_history(delete_names: list[list[str]],
	install_names: list[list[str]], upgrade_names: list[list[str]]) -> None:
	"""Prepare history for writing."""
	# We don't need only downloads in the history
	if '--download-only' in sys.argv[1:]:
		return
	timezone = datetime.utcnow().astimezone().tzinfo
	time = datetime.now().strftime('%Y-%m-%d %H:%M:%S')+' '+str(timezone)
	history_dict = load_history_file() if NALA_HISTORY.exists() else {}
	hist_id = str(len(history_dict) + 1 if history_dict else 1)
	altered = len(delete_names) + len(install_names) + len(upgrade_names)

	transaction: dict[str, str | list[str] | list[list[str]]] = {
		'Date' : time,
		'Command' : sys.argv[1:],
		'Altered' : str(altered),
		'Removed' : delete_names,
		'Installed' : install_names,
		'Upgraded' : upgrade_names,
	}

	history_dict[hist_id] = transaction
	write_history_file(history_dict)

def get_history(hist_id: str) -> dict[str, str | list[str] | list[list[str]]]:
	"""Get the history from file."""
	dprint(f"Getting history {hist_id}")
	if not NALA_HISTORY.exists():
		sys.exit("No history exists..")
	history_file: dict[str, dict[str, str | list[str] | list[list[str]]]] = json.loads(NALA_HISTORY.read_text(encoding='utf-8'))
	transaction = history_file.get(hist_id)
	if transaction:
		return transaction
	sys.exit(ERROR_PREFIX+f"Transaction {hist_id} doesn't exist.")