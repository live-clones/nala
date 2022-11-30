#!/bin/sh
set -e

# This is more simple as a shell script than directly
# in the Makefile due to python venv. It needs to activate and deactivate

# Install pyinstaller outside the workspace
deactivate || echo "Already deactivated"
python3 -m pip install pyinstaller -U

# Activate the virutal environment first
python3 -m venv ./.venv
. ./.venv/bin/activate

# Install Nala dependencies
python3 -m pip install ./
poetry install --no-dev

# make sure directories are clean
rm -rf ./build/ ./dist/ ./**/__pycache__/

pyinstaller --noconfirm \
--clean \
--console --nowindowed --noupx \
--paths "./.venv/lib/site-packages" \
--paths "/usr/lib/python3/dist-packages" \
--paths "/lib/python3/dist-packages/" \
--exclude-module IPython \
--exclude-module IPython.display \
--exclude-module IPython.core \
--exclude-module IPython.core.formatters \
--exclude-module ipywidgets \
--exclude-module java \
--exclude-module java.lang \
--exclude-module winreg \
--exclude-module _winreg \
--exclude-module _winapi \
--exclude-module win32api \
--exclude-module win32com \
--exclude-module win32com.shell \
--exclude-module msvcrt \
--name nala \
./nala/__main__.py

# Remove the excluded modules from the warnings list
sed -i '/excluded module /d' ./build/nala/warn-nala.txt

# Smoke test
./dist/nala/nala --help

# Archive the build and deactivate the virtual env
cd ./dist && tar cv nala/ | xz -9 > ./nala.tar.xz
deactivate

# TODO add docs to the pyinstaller
# --add-data="README.rst:." \
# --add-data="docs:docs" \
