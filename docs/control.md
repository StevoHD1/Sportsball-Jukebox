# External control: MIDI, OSC, Stream Deck and the phone remote

Pads can be fired from a keyboard, MIDI controllers, the network (OSC),
Stream Deck, and any phone or tablet browser. External triggers are handled on their own threads and
go straight to the engine. They don't wait for the UI, so a MIDI pad
fires within a millisecond or so.

## Keyboard

- Each pad has a key, shown on the pad and set in its editor.
- Keys apply to the **bank shown**, so different banks can reuse the same
  keys.
- **Esc** fades out everything (Stop All).
- **⌘1–⌘9** switch banks.

## MIDI

MIDI is on by default and listens to every connected MIDI input. Set it up
in **Show → Control…** (⇧⌘M).

- **Assign a pad:** right-click it → **Learn MIDI Trigger**, then press a
  key, pad or button on the controller. The pad gets a 🎹 badge. Learning
  also works from the pad editor.
- **Show-wide controls:** Stop All, Next Bank and Previous Bank can be
  learned in the Control window.
- **What triggers:** a note-on (velocity above 0), a control change with a
  value above 0 (a button press, not its release), or a program change.
  Triggers learned from a controller remember its MIDI channel.
- **New devices:** after plugging in a controller, press **Rescan**.

The bottom bar of the main window shows the last control message and what
it did, which helps when setting up.

## OSC

Turn OSC on in **Show → Control…** and pick a UDP port (default
53100). Any OSC sender on the network can then control the show:

| Address | Action |
|---|---|
| `/sbj/fire/<pad>` | Fire a pad in the bank shown (pads count from 1) |
| `/sbj/fire/<bank>/<pad>` | Fire a pad in a specific bank |
| `/sbj/fire` + int argument | Fire that pad in the bank shown |
| `/sbj/go/<pad>`, `/sbj/go/<bank>/<pad>` | Same as fire. On a vamping pad it lets the music continue, which is clearer on a Stream Deck button |
| `/sbj/title` + string argument | Fire the pad with that title (any case) |
| `/sbj/next/<pad>`, `/sbj/next/<bank>/<pad>` | Playlist pad: crossfade to the next song |
| `/sbj/stop/<pad>`, `/sbj/stop/<bank>/<pad>` | Stop a pad |
| `/sbj/fade/<pad>`, `/sbj/fade/<bank>/<pad>` | Fade a pad out (over the pad's fade-out time, 0.5 s by default) |
| `/sbj/stopall` | Fade everything out (over the show's Stop All fade, 0.5 s by default) |
| `/sbj/stopall/now` | Stop everything immediately |
| `/sbj/bank/<n>`, `/sbj/bank/next`, `/sbj/bank/prev` | Change bank |

A message whose first argument is `0` is ignored. Controllers like
TouchOSC send 1 when a button is pressed and 0 when it's released, and
only the press should act.

Test from a terminal on another Mac (with `oscsend` from liblo, or any
OSC tool):

```sh
oscsend <mac-ip> 53100 /sbj/fire/3
```

## Phone and tablet remote

The app can serve a web page on the local network, so a producer or a
second operator can fire pads from courtside. Any phone or tablet browser
works; there's nothing to install.

1. **Show → Control…** → **Phone & tablet remote**: turn on "Serve the
   remote" (TCP port 8765 by default).
2. On a phone on the **same Wi-Fi**, scan the QR code. It opens the page
   and signs in with the PIN. Or type the address shown
   (`http://<mac-ip>:8765/`) and enter the 6-digit PIN.
3. The page shows the show's banks, the pads of the selected bank with
   their colours, progress and VAMP state, a Now Playing list with
   countdowns and Stop buttons (and GO for a vamp), and a big STOP ALL.

How it behaves:
- Presses act like OSC: they're logged with the source `remote`, appear
  in the bottom bar as "Remote …", and fire straight into the engine.
- Switching banks on the remote switches the app's bank too (there's one
  shared "bank shown"). Pads are fired by their absolute slot, so a press
  never hits the wrong bank.
- The page refreshes about twice a second. Its state is a snapshot, so the
  countdown on the phone can lag the Mac by a few hundred milliseconds.
- **Security:** every `/api/` request must carry the PIN. A wrong PIN
  waits 300 ms, one at a time across all connections, so guessing a
  6-digit PIN would take days. **New PIN** locks out every phone until
  it's given the new one. The traffic is plain HTTP, so use it only on a
  network you trust (the production Wi-Fi, not public guest Wi-Fi), and
  turn it off when not needed. It's off by default.
- If the Mac's firewall is on, macOS asks once whether to allow incoming
  connections: choose **Allow**.

API (for scripts and custom controllers; send header `X-SBJ-PIN: <pin>`):

| Request | Action |
|---|---|
| `GET /api/state` | Show name, banks, current bank, the bank's pads, playing pads |
| `POST /api/fire/<slot>` | Fire (or GO on a vamp). Slot = bank × pads per bank + pad, from 0 |
| `POST /api/stop/<slot>`, `/api/fade/<slot>`, `/api/next/<slot>` | Stop, fade out, next song |
| `POST /api/stopall`, `/api/stopall/now` | Stop All (fade), or immediately |
| `POST /api/bank/<n>`, `/api/bank/next`, `/api/bank/prev` | Change bank (from 1) |

Every successful request returns the new state as JSON.

To work on the page without a Mac: `cargo run -p sbj-control --example
remote_demo`, then open `http://127.0.0.1:8765/#pin=123456`.

## Stream Deck

There are three ways, from most to least robust:

1. **OSC plugin** (e.g. an "OSC Sender" plugin from the Stream Deck
   store): point it at the Mac's IP and port 53100, and send
   `/sbj/fire/<pad>`, `/sbj/title` with a pad's name, or `/sbj/stopall`.
   This works even when Sportsball Jukebox isn't the frontmost app.
2. **MIDI plugin**: send a note per button, then use Learn MIDI Trigger on
   each pad.
3. **Hotkey action**: send the pad's key. This only works while Sportsball
   Jukebox is the frontmost app.

## QLab and other show controllers

QLab's Network cues can send the OSC messages above. Use a different port
from QLab's own (53000); the default 53100 already is.
