#!/bin/sh
set -e

# This is more simple as a shell script than directly
# in the Makefile due to python venv. It needs to activate and deactivate

# Required system dependencies
sudo apt-get install devscripts apt-utils -y

# Install pyinstaller outside the workspace
deactivate || echo "Already deactivated"
python3 -m pip install pyinstaller -U

# Activate the virutal environment first
python3 -m venv ./.venv
. ./.venv/bin/activate

# Install Nala dependencies
python3 -m pip install ./
poetry install

# make sure directories are clean
rm -rf ./build/ ./dist/ ./**/__pycache__/

# Get the venv paths list
venv_paths=$(python3 -c '
from site import getsitepackages
from os import path, curdir

paths = getsitepackages()
args = map(lambda pth: f"--paths {path.relpath(pth, start = curdir)}", paths)
print(" ".join(args))
')

# Get the system paths list
system_paths=$(sudo python3 -c '
from site import getsitepackages

paths = getsitepackages()
args = map(lambda pth: f"--paths {pth}", paths)
print(" ".join(args))
')

pyinstaller --noconfirm \
    --clean \
    --console --nowindowed --noupx \
    $venv_paths \
    $system_paths \
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
cd ./dist && tar cv nala/ | xz -9 >./nala.tar.xz
deactivate

# TODO add docs to the pyinstaller
# --add-data="README.rst:." \
# --add-data="docs:docs" \