export function processListContainsApp(processList, targetApp, mainBinary) {
  const expected = `${targetApp}/Contents/MacOS/${mainBinary}`;
  const translocated = `${targetApp.split('/').at(-1)}/Contents/MacOS/${mainBinary}`;
  return processList
    .split('\n')
    .map((line) => line.trim())
    .some((command) => {
      if (command === expected || command.startsWith(`${expected} `)) return true;
      // Quarantine hides the original app path, but preserves its exact bundle name.
      const prefix = /^\/(?:private\/)?var\/folders\/[^/]+\/[^/]+\/T\/AppTranslocation\/[^/]+\/d\//.exec(command);
      if (!prefix) return false;
      const relative = command.slice(prefix[0].length);
      return relative === translocated || relative.startsWith(`${translocated} `);
    });
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
