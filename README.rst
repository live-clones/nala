# Nala
======
A wrapper for the apt package manager.

# Installation
==============

Install the Volian Scar repo and then install Nala

.. code-block:: console

	echo "deb http://deb.volian.org/volian/ scar main" | sudo tee /etc/apt/sources.list.d/volian-archive-scar-unstable.list
	wget -qO - https://deb.volian.org/volian/scar.key | sudo tee /etc/apt/trusted.gpg.d/volian-archive-scar-unstable.gpg > /dev/null
	sudo apt update && sudo apt install nala

If you want to add the source repo

.. code-block:: console

	echo "deb-src http://deb.volian.org/volian/ scar main" | sudo tee -a /etc/apt/sources.list.d/volian-archive-scar-unstable.list

There isn't a documentation site setup at the moment, but our man page explains things well enough for now.

# Todo
======

**Commands and Switches**

- -f --fix-broken
- --no-install-recommends
- --install-suggests
- Nala download
- Probably many others to add as well

**Internal**

- Implement optional bandwidth check on fetch
- Remove exceptions for just error messages
- Setup ReadTheDocs pages
