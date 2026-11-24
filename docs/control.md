# External control: MIDI, OSC and Stream Deck

Pads can be fired from a keyboard, MIDI controllers, the network (OSC),
and Stream Deck. External triggers are handled on their own threads and
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
in **Show → MIDI & OSC…** (⇧⌘M).

- **Assign a pad:** right-click it → **Learn MIDI Trigger**, then press a
  key, pad or button on the controller. The pad gets a 🎹 badge. Learning
  also works from the pad editor.
- **Show-wide controls:** Stop All, Next Bank and Previous Bank can be
  learned in the MIDI & OSC window.
- **What triggers:** a note-on (velocity above 0), a control change with a
  value above 0 (a button press, not its release), or a program change.
  Triggers learned from a controller remember its MIDI channel.
- **New devices:** after plugging in a controller, press **Rescan**.

The bottom bar of the main window shows the last control message and what
it did, which helps when setting up.

## OSC

Turn OSC on in **Show → MIDI & OSC…** and pick a UDP port (default
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
