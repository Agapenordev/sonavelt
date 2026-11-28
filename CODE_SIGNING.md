# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).

The Windows program (`sonavelt.exe`) is signed so Windows can tell it comes from this project and
hasn't been changed. Only programs built from this repository's source code are signed: the
[Release workflow](.github/workflows/release.yml) builds them on GitHub's own runners, and every
signing request is approved by hand before it is signed. Third-party programs that Sonavelt uses,
such as ffmpeg and yt-dlp, are not part of the download and are not signed by this project.

## Team

| Role | Who |
|---|---|
| Committers and reviewers | [Agapenordev](https://github.com/Agapenordev) |
| Approvers | [Agapenordev](https://github.com/Agapenordev) |

- **Committers** may change the source code without further review.
- **Reviewers** review every change from anyone else (pull requests) before it is merged.
- **Approvers** approve each signing request.

Everyone on the team uses two-factor authentication on GitHub and SignPath.

## Privacy

Sonavelt has no accounts, no telemetry and no server of its own. Your library, playlists, history
and settings stay in a file on your computer. To do its job it connects to these services:

| Service | When | What it receives |
|---|---|---|
| YouTube (through yt-dlp) | You search or play a YouTube song; the Home page finds picks | Your searches, and the videos to play or find similar songs for |
| [radio-browser.info](https://www.radio-browser.info) | You open Radio or search stations | Genre or station names you search for |
| Radio stations | You play a station; station logos are looked up | The usual request to play the stream or load the station's website |
| [LRCLIB](https://lrclib.net) | A song starts playing | The song's title and artist, to find its lyrics |
| [GitHub](https://github.com/Agapenordev/sonavelt/releases) | At start, to check for a new version; when you install one | Only the request for the latest release |

Each service handles requests under its own privacy policy:
[YouTube / Google](https://policies.google.com/privacy),
[radio-browser.info](https://www.radio-browser.info/faq),
[LRCLIB](https://lrclib.net),
[GitHub](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement).
