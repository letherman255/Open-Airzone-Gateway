import os
import re
import subprocess
import sys

REPO = os.path.join(os.path.dirname(__file__), "..", "..")


def test_openapi_up_to_date():
    r = subprocess.run([sys.executable, "-I", os.path.join(REPO, "tools", "gen_openapi.py"), "--check"], capture_output=True, text=True)
    assert r.returncode == 0, r.stdout + r.stderr


def test_every_api_route_documented():
    api = open(os.path.join(REPO, "firmware", "core", "src", "api.rs"), encoding="utf-8").read()
    spec = open(os.path.join(REPO, "docs", "openapi.yaml"), encoding="utf-8").read()
    for first in set(re.findall(r'\["([a-z]+)"', api)):
        assert f"  /{first}" in spec, first


def test_workflows_parse():
    import yaml
    for name in ("ci.yml", "release.yml"):
        with open(os.path.join(REPO, ".github", "workflows", name), encoding="utf-8") as f:
            data = yaml.safe_load(f)
        assert "jobs" in data, name


def test_doc_links_resolve():
    for root, _, files in os.walk(REPO):
        if any(p in root for p in ("target", ".git", ".venv", "node_modules", ".superpowers")):
            continue
        for fn in files:
            if not fn.endswith(".md"):
                continue
            path = os.path.join(root, fn)
            text = open(path, encoding="utf-8").read()
            for link in re.findall(r"\]\(([^)#\s]+)\)", text):
                if link.startswith(("http://", "https://", "mailto:")):
                    continue
                assert os.path.exists(os.path.normpath(os.path.join(root, link))), f"{path}: {link}"
