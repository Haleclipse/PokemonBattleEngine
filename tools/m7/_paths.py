"""Shared by the tools/m7 scripts: WT (the checkout being worked on) and MAIN (the main repository,
home of target/m8 and work/), from git — or from the WT / MAIN environment variables."""
import os
import subprocess


def _git(*args):
    return subprocess.run(['git', *args], capture_output=True, text=True, check=True).stdout.strip()


WT = os.path.abspath(os.environ.get('WT') or _git('rev-parse', '--show-toplevel'))
# `--show-toplevel` in a worktree names the worktree; the common git directory sits in the main
# repository.
MAIN = os.path.abspath(os.environ.get('MAIN') or os.path.dirname(
    _git('-C', WT, 'rev-parse', '--path-format=absolute', '--git-common-dir')))
