#!/bin/sh

# This is more simple as a shell script than directly
# in the Makefile due to python venv. It needs to activate and deactivate

# Activate the virutal environment first
python3 -m venv ./.venv
. ./.venv/bin/activate

python3 -m pip install pyinstaller

# Install Nala and make sure directories are clean
python3 -m pip install ./
rm -rf ./build/ ./dist/

pyinstaller --noconfirm \
--nowindow --noupx \
--paths ./.venv/lib/site-packages \
./nala/nala.py

# Archive the build and deactivate the virtual env
cd ./dist && tar cv ./nala/ | xz -9 > ./nala.tar.xz
deactivate

# TODO add docs to the pyinstaller
# --add-data="README.rst:." \
# --add-data="docs:docs" \
