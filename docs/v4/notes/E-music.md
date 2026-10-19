# E: music (stream E notes)

## M1: play/pause shown reversed (fix)

User report 2026-10-19: "when I have it playing, it actually shows paused, and when I have a pause, it actually shows it's playing"
(YouTube Music, with Nocturne running).

**Cause (verified in code):** the music card drew the *state* (a triangle while playing, two bars while paused). Every player, and
CoolerScreen's own preview button, draws the *button* convention: two bars while playing, a triangle while paused. So the card looked
reversed whatever app was playing. Not a Windows status problem: GSMTC `Playing`/`Paused` were mapped correctly.

**Fix:** `live/render/widgets.rs` `transport_icon()`: two bars while playing, triangle while paused. Same size, place and colour.
No setting: this is a bug fix, and existing themes otherwise look the same.

**Which player the card follows when several apps have a media session** (`live/nowplaying.rs` `choose()`), in order:
1. a preferred app, if one is set (internal `MusicConfig.preferred_app`; nothing in the UI sets it yet, so normally skipped);
2. Windows' current media session (the one the Windows volume flyout shows), if it is playing;
3. otherwise the first session that is playing;
4. otherwise Windows' current session (paused; the card stays for a minute after pausing, as before);
5. nothing: the card hides as before.

Before, step 2 did not exist: with two players playing at once the card followed whichever Windows listed first.

**Probe of this PC (read only, 2026-10-19):** two sessions from the same app id `308046B0AF4A39CB` (a hashed id, as browsers and
packaged apps use): one Playing ("Raindance", Dave and Tems), one Paused (a second track). The card follows the playing one, under the
old and the new rule alike, so the reversal the user saw was the icon. No session named Nocturne or a test player was present at that time.

**Known, left as is:** event hooks are keyed by app id, so when two sessions share one id, switching between them is picked up by the
1 s poll rather than an event (up to 1 s late). Harmless.

**Tests:** `transport_icon` unit test; a render test (`music_icon_tests`) drawing the card playing and paused and checking two bars
vs one shape in the icon area; five `choose` tests (preferred wins even when paused, two playing → the current one, current paused +
other playing → the playing one, none playing → current, empty → none). Full lib suite: 337 passed, 0 failed, 19 ignored.

Changelog line (for the PM): "Music: the play/pause sign now shows like a player's button (two bars while playing); with several players
open, the one Windows shows as current wins when it is playing."
