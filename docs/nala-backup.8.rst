===========
nala-backup
===========

-------------------------------------------------------------
backup files to migrate system configuration to a new install
-------------------------------------------------------------

:Date: 23 May 2022
:Copyright: Copyright (C) 2021, 2022 Blake Lee
:Version: 0.9.1
:Manual section: 8
:Manual group: NALA

SYNOPSIS
========

``nala backup`` [`--options`]

DESCRIPTION
===========

By default ``nala`` will ``backup``:

	Packages which were explicitly installed with ``nala``.

	The ``nala`` `/etc/nala/nala.conf` file.

	The ``apt`` `/etc/apt/apt.conf` file.

	The ``apt`` `/etc/apt/preferences` file.

Example:

	If you would like to include only the packages that
	``apt list`` `--manually-installed` outputs

		``nala backup`` `--apt-installed`

Example:

	You might want to include the preferences directory
	and even include auto-installed packages (It's a lot)

		``nala backup`` `--pref-dir --all-installed`


OPTIONS
=======

--nala, --no-nala

	Backup the `/etc/nala/nala.conf` configuration file.

	[default: --nala]

--apt, --no-apt

	Backup the `/etc/apt/apt.conf` configuration file.

	[default: --apt]

--apt-dir, --no-apt-dir

	Backup the `/etc/apt/apt.conf.d/` configuration directory.

	[default: --no-apt-dir]

--pref, --no-pref

	Backup the `/etc/apt/preferences` configuration file.

	[default: --pref]

--pref-dir, --no-pref-dir

	Backup the `/etc/apt/preferences.d/` configuration directory.

	[default: --no-pref-dir]

--apt-installed

	Include only manually installed packages from ``apt list`` `--manually-installed`

--manually-installed

	Similar to `--apt-installed` but does not include packages with a priority of either:

		`essential`, `required`, `standard`, `important`

	Takes precedence over `--apt-installed`

--all-installed

	Include all installed packages even if they're auto-installed

	`--apt-installed` overrides:

		`--manually-installed`

		`--apt-installed`

--debug
	Print helpful information for solving issues.
	If you're submitting a bug report try running the command again with `--debug`
	and providing the output to the devs, it may be helpful.

-v, --verbose
	Disable scrolling text and print extra information

-h, --help
	Shows this man page.
