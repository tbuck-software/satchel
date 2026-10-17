export function processListContainsApp(processList, targetApp, mainBinary) {
  const expected = `${targetApp}/Contents/MacOS/${mainBinary}`;
  return processList
    .split('\n')
    .map((line) => line.trim())
    .some((command) => command === expected || command.startsWith(`${expected} `));
}

export function stableInstallProblem({ channel, branch, status, allowUnstable }) {
  if (channel !== 'stable' || allowUnstable) return null;
  if (branch !== 'main' || status.trim()) {
    return 'Stable installs require a clean main checkout. Use the daily channel, or pass --allow-unstable-stable-install to deliberately override the guard.';
  }
  return null;
}

export function versionTagProblem(tag, existingRevision, revision) {
  return existingRevision && existingRevision !== revision
    ? `${tag} already identifies another commit. Bump the release version before installing; release tags are immutable.`
    : null;
}
