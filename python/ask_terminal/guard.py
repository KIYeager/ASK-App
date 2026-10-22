"""Ordnersperre für die Datei-API von open-terminal.

open-terminal löst jeden Pfad aus einer Anfrage mit ``UserFS.resolve_path`` auf. Hier wird
diese Methode so erweitert, dass nur Pfade innerhalb des Arbeitsordners durchkommen.
Verknüpfungen (Junctions, Symlinks) werden vorher aufgelöst, damit sie nicht hinausführen.
"""
import os


class PathGuard:
    def __init__(self, root: str):
        self.display_root = os.path.realpath(root)
        self.root = os.path.normcase(self.display_root).rstrip("\\/")

    def inside(self, path: str) -> bool:
        try:
            resolved = os.path.realpath(path)
        except (OSError, ValueError):
            return False
        resolved = os.path.normcase(resolved)
        return resolved == self.root or resolved.startswith(self.root + os.sep)

    def check(self, path: str) -> None:
        if not self.inside(path):
            raise PermissionError(
                f"Zugriff verweigert: {path} liegt ausserhalb des Arbeitsordners {self.display_root}"
            )


def install(user_fs_class, guard: PathGuard) -> None:
    """Hängt die Sperre in UserFS ein (resolve_path, _check_path, is_path_allowed)."""
    original_resolve = user_fs_class.resolve_path
    original_check = user_fs_class._check_path
    original_allowed = user_fs_class.is_path_allowed

    def resolve_path(self, path, cwd=None):
        target = original_resolve(self, path, cwd)
        guard.check(target)
        return target

    def _check_path(self, path):
        original_check(self, path)
        guard.check(path)

    def is_path_allowed(self, path):
        return original_allowed(self, path) and guard.inside(path)

    user_fs_class.resolve_path = resolve_path
    user_fs_class._check_path = _check_path
    user_fs_class.is_path_allowed = is_path_allowed
