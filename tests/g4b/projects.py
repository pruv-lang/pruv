"""G4b differential fixtures: small manifest workspaces under tests/g4b/projects/<name>/.

Each workspace holds sibling package directories; the program package is `app/` (`app/pruv.json` plus its root file) and
dependencies are `../<pkg>`.

  all_projects() -> [(name, manifest_path_abs, root_path_abs)]   sorted by name
  copy_workspace(name, dest) -> (manifest, root)                  copies the whole tree to dest, returns the new paths
"""
import json
import os
import shutil

HERE = os.path.dirname(os.path.abspath(__file__))
PROJECTS = os.path.join(HERE, "projects")
MANIFEST = os.path.join("app", "pruv.json")


def _root_of(manifest):
    with open(manifest, encoding="utf-8") as fh:
        spec = json.load(fh)
    return os.path.join(os.path.dirname(manifest), *spec["root"].split("/"))


def all_projects():
    out = []
    for name in sorted(os.listdir(PROJECTS)):
        manifest = os.path.join(PROJECTS, name, MANIFEST)
        if os.path.isfile(manifest):
            out.append((name, manifest, _root_of(manifest)))
    return out


def copy_workspace(name, dest):
    """Copy tests/g4b/projects/<name>/ to dest (created; an existing dest is merged). -> (manifest, root) in dest."""
    shutil.copytree(os.path.join(PROJECTS, name), dest, dirs_exist_ok=True)
    manifest = os.path.abspath(os.path.join(dest, MANIFEST))
    return manifest, _root_of(manifest)
