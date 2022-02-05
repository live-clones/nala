# Nala
======
Nala is a front-end for ``libapt-pkg``. Specifically we interface using the ``python-apt`` api.

Especially for newer users it can be hard to understand what ``apt`` is trying to do when installing or upgrading.

We aim to solve this by not showing some redundant messages, formatting the packages better, and using color to
show specifically what will happen with a package during install, removal, or an upgrade.

.. image:: /imgs/nala-install-1.png
.. image:: /imgs/nala-install-2.png

# Parallel Downloads
====================
Outside of pretty formatting, the number 1 reason to use Nala over ``apt`` is parallel downloads.

``apt`` downloads 1 package at a time, where as we download multiple.
By default we will download 2 packages per unique mirror in your ``sources.list`` file, up to a maximum of 16.
Theoretically Nala can download 16x faster than ``apt``.
We have the 2 thread per mirror limit to minimize how hard we are hitting mirrors.
Additionally we alternate downloads between the available mirrors to improve download speeds even further.
If a mirror fails for whatever reason, we just try the next until all defined mirrors are exhausted.

# Fetch
=======
Which brings us to our next standout feature, ``nala fetch``.

This command works similar to how most people use ``netselect`` and ``netselect-apt``.
``nala fetch`` will check if your distro is either Debian or Ubuntu.
Nala will then go get all the mirrors from the respective master list.
Once done we test the latency and score each mirror.
Nala then will choose the fastest 3 mirrors (configurable) and write them to a file.

`At the moment fetch will only work on Debian, Ubuntu and derivatives still tied to the main repos. Such as Pop!_OS`

.. image:: /imgs/nala-fetch.png

# History
=========
Our last big feature is the ``nala history`` command.

If you're familiar with ``dnf`` this works much in the same way.
Each Install, Remove or Upgrade we store in /var/lib/nala/history.json with a unique ``<ID>`` number.
At any time you can call ``nala history`` to print a summary of every transaction ever made.
You can then further manipulate this with commands such as ``nala history undo <ID>`` or ``nala history redo <ID>``.
If there is something in the history file that you don't want you can use the ``nala history clear <ID>`` It will remove that entry.
Alternatively for the ``clear`` command we accept ``all`` as an argument which will remove the entire history.

.. image:: /imgs/nala-history-info.png
.. image:: /imgs/nala-history-undo.png

# Installation
==============

**Volian Scar Repo**

Install the Volian Scar repo and then install Nala.

.. code-block:: console

	echo "deb http://deb.volian.org/volian/ scar main" | sudo tee /etc/apt/sources.list.d/volian-archive-scar-unstable.list
	wget -qO - https://deb.volian.org/volian/scar.key | sudo tee /etc/apt/trusted.gpg.d/volian-archive-scar-unstable.gpg > /dev/null
	sudo apt update && sudo apt install nala

If you want to add the source repo.

.. code-block:: console

	echo "deb-src http://deb.volian.org/volian/ scar main" | sudo tee -a /etc/apt/sources.list.d/volian-archive-scar-unstable.list

**Pacstall**

Alternatively, we maintain a pacscript for ``Pacstall``.

If you don't already, install `Pacstall <https://github.com/pacstall/pacstall>`_.

Once that is complete all you have to do is run the following command.

.. code-block:: console

	pacstall -I nala-deb

**Debian Package**

You can also choose to download our ``.deb`` and install it locally through `apt` or `dpkg`.

To download the package you can head over to our `Releases <https://gitlab.com/volian/nala/-/releases>`_ page.

From there you can use one of the two commands below to install ``nala``.

.. code-block:: console

	sudo apt install /path/to/nala_version_arch.deb

Or

.. code-block:: console

	sudo dpkg -i /path/to/nala_version_arch.deb
	sudo apt install -f

There isn't a documentation site setup at the moment, but our man page explains things well enough for now.

# Additional Images
===================

.. image:: /imgs/nala-update.png
.. image:: /imgs/nala-show-apt.png

# Bug Reports or Feature Requests
=================================
Nala is mirrored to several sites such as GitHub and even Debian Salsa.

The official repository is https://gitlab.com/volian/nala

We ask that you please go here to report a bug or request a feature.

The other repositories are official, but just mirrors of what is on GitLab.
