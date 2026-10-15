// Where to read the third-party licence texts when the app can't open the copy it carries.

const REPO = "https://github.com/perritodeck/perritodeck";

/** A release's own texts: release.yml attaches third-party-licenses.txt to the release of
 * tag v<version>. A dev build (`tauri dev`) may be ahead of any release, so it gets the
 * notices on main. A local `tauri build` counts as a release here. */
export function licencesUrl(version: string, dev: boolean): string {
  return dev
    ? `${REPO}/blob/main/THIRD_PARTY_NOTICES.md`
    : `${REPO}/releases/download/v${version}/third-party-licenses.txt`;
}
