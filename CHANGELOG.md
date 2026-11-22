# Changelog

## 1.0.3 — 2026-11-22

- The Windows installer now asks whether to install for everyone on the computer or only for you. "Only for you" needs no administrator prompt and installs to your own Programs folder. An update keeps the choice and folder of the previous install, and the uninstaller removes what that install wrote
- The installer accepts /ALLUSERS and /CURRENTUSER on the command line for a silent install
- Settings opens faster: the General and About pages are built the first time you open them

## 1.0.2 — 2026-11-15

- New traffic limits: a named set holds a speed limit shared by all the downloads and how many run at once. Pick one with the new Traffic drop-down in the panel, and edit the sets in Settings > Traffic limits (new, duplicate, delete, drag to reorder). Two sets come with the app: Unlimited (2 at once) and Background (500 KB/s, 1 at once)
- The number of downloads at the same time is now part of the traffic set; the number you had chosen is kept in the Unlimited set
- New Pause button while downloads run: the running downloads freeze and the waiting ones do not start; Resume lets them go on. After a long pause a download starts again from the part of the file it already has
- Download, Pause, Resume and Stop are buttons with their name instead of icons
- New `track` name for the file name of a destination: the number of the file in its album or playlist, with at least two digits. A file that has no number is named without it, and without the dash or space that joined it
- A click on a name in the File name or Sub-folder name box puts it where the caret is (or over the selection) instead of at the end
- Each list has its own player: what plays under Video and what plays under Audio keep their own file, position and picture
- The row whose file is in the player is highlighted, and the player's title shows the track number as the list does
- The broom removes the finished downloads of the list shown only; the other list keeps its own
- The button that opens a finished file in FFmpeg Video Studio or FFmpeg Audio Studio shows that app's icon
- The player's position slider is a thick line with a red mark for the current moment
- A link being looked up reads "Analyzing…" instead of "Looking up…"
- The button that opened the download folder is removed from the panel
- The window cannot be made smaller than 640 × 400
- Closing the window while downloads are paused stops them
- Build: the macOS job installs a dx built for its own processor and checks that it starts

## 1.0.1 — 2026-11-14

- A finished video plays with its picture: the player shows the video above the list, for every format ffmpeg reads
- The player is a pane above the list instead of a bar in the footer: the title, the picture of a video, the two times, the position slider on the full width, then one row of buttons (previous, play or pause, stop, next, volume, close)
- The player belongs to the list its file is in: with the other list shown it is hidden and paused, and it is back, paused at the same moment, when you return
- A finished row has a new button, Open in FFmpeg Video Studio (Video list) or Open in FFmpeg Audio Studio (Audio list), shown when that app is installed in version 2.0.2 or later; it starts the app with the file
- The Video and Audio lists are chosen with two buttons at the start of the header instead of a tab strip above the list
- New reminder button in the title bar when a newer version is out: it stays after the new-version card is closed or hidden with Don't show again, and a click reopens the card with the release notes and the download link
- New News button in the title bar while yaiol has an announcement for the app: a click reopens the announcements, including the ones hidden with Don't show again. It does not appear when Show news from yaiol is off
- Don't show again can be undone: reopen a card from its title-bar button and untick the box, and it shows again at the next launch
- The About page describes the app (it showed a placeholder sentence), and the Settings button's tooltip reads "Settings" instead of "Open settings"
- The player's two sliders have a new look
- README rewritten, with a logo

## 1.0.0 — 2026-11-12

- Initial release
