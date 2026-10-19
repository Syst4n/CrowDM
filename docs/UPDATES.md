# Updates and public download counts

Source commits and preview releases do not announce an update to installed apps.
The app checks `updates/windows-x64.json` once daily when automatic checks are
enabled, or on demand in Settings → Updates and downloads. A null `release`
means no update has been announced. Keep it null during preview development.

Only when the owner explicitly says **send this update to users**:

1. Build, test and publish the chosen version and matching extension assets.
2. Prepare a JSON array of short, plain-text “What’s New” items.
3. Run `node scripts/announce-update.mjs VERSION WHATS_NEW.json`. It checks
   the published installer name, asset digest and extension version.
4. Review and separately commit/push the announcement manifest. This action
   makes the version eligible for the app's daily notice.

The app validates the version, exact repository/asset URL and SHA-256 digest.
Choosing Download verified installer creates a normal checksum-verified
CrowDM job. Installing remains a user action. There is no unattended installer
execution or signed automatic updater in this preview. Keep the GitHub account
and repository protected: the announcement feed shares their trust boundary.

The bundled extension version is shown alongside connected versions. Unpacked
Chromium extensions need manual reload; temporary Firefox add-ons must be loaded
again. The app explains the steps and opens its extension folder. Store-managed
extension updates require future Chrome/Edge/Firefox store publication.
The 0.3 extension adds the selected-link button; reload the matching updated
extension once and refresh already-open pages to receive that content script.

The community counter retrieves public GitHub installer and portable asset
download counts, grouped by release version. These are downloads, not unique
people or installations. Repeated downloads and automated requests can count.
The query is manual, can fail offline or under GitHub rate limits, and examines
up to 1,000 releases. Active users and installed-version distribution are not
available. CrowDM has no usage-reporting endpoint or installation tracking ID.
