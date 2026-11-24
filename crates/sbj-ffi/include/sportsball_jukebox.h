/*
 * Sportsball Jukebox engine C API.
 *
 * Keep in sync with crates/sbj-ffi/src/lib.rs.
 *
 * Conventions:
 *  - Functions returning SbjStatus report failures there; the reason is
 *    available from sbj_last_error_message() on the same thread.
 *  - All functions tolerate NULL handles (SBJ_STATUS_NULL_POINTER).
 *  - A session may be used from any thread. Slow work (decoding audio)
 *    happens outside its lock, so it never blocks firing cues; still, call
 *    the functions marked SLOW off the main thread.
 *  - Shows, cues and outputs cross as JSON in the show-file schema
 *    (crates/sbj-session/src/show.rs). Returned strings must be freed with
 *    sbj_string_free().
 *  - Strings are NUL-terminated UTF-8. Times are in seconds.
 */
#ifndef SPORTSBALL_JUKEBOX_H
#define SPORTSBALL_JUKEBOX_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum SbjStatus {
    SBJ_STATUS_OK = 0,
    SBJ_STATUS_NULL_POINTER = 1,
    SBJ_STATUS_INVALID_ARGUMENT = 2,
    SBJ_STATUS_QUEUE_FULL = 3,
    SBJ_STATUS_INTERNAL_ERROR = 4,
    SBJ_STATUS_NO_OUTPUT = 5,   /* the cue's output device is not available */
    SBJ_STATUS_DEVICE_ERROR = 6,
    SBJ_STATUS_MEDIA_ERROR = 7,
    SBJ_STATUS_TRUNCATED = 8,
    SBJ_STATUS_SHOW_ERROR = 9,  /* show file unreadable or change invalid */
} SbjStatus;

#define SBJ_ID_LEN 512
#define SBJ_NAME_LEN 256

typedef struct SbjDeviceInfo {
    char id[SBJ_ID_LEN];     /* persistent; use in output JSON */
    char name[SBJ_NAME_LEN]; /* for display */
    uint16_t output_channels;
    uint32_t sample_rate;
    bool is_default;
} SbjDeviceInfo;

typedef struct SbjCueState {
    bool assigned;  /* the pad has a cue */
    bool playing;
    bool looping;
    bool ready;     /* loaded on an open output: can fire */
    bool holding;   /* vamping: firing again lets it continue */
    double position_seconds;
    double duration_seconds;
} SbjCueState;

typedef struct SbjSession SbjSession;

/* ---- General ---------------------------------------------------------- */

const char *sbj_version(void);

/* Copies the last error message on this thread into buf (NUL-terminated
 * when len > 0). Returns the full message length, excluding the NUL. */
size_t sbj_last_error_message(char *buf, size_t len);

/* Frees a string returned by this library. */
void sbj_string_free(char *s);

/* True if this header's struct layouts match the library's. Call at
 * startup: sbj_abi_check(sizeof(SbjDeviceInfo), sizeof(SbjCueState)). */
bool sbj_abi_check(size_t device_info_size, size_t cue_state_size);

/* ---- Session ---------------------------------------------------------- */

/* Starts with an empty show and no devices open. */
SbjSession *sbj_session_create(void);
void sbj_session_destroy(SbjSession *session);

/* Opens every output's device (preferred, else backup, else default). */
SbjStatus sbj_session_open_outputs(const SbjSession *session);

/* Housekeeping and automatic device failover: call every ~30 ms. */
SbjStatus sbj_session_poll(const SbjSession *session);

SbjStatus sbj_session_backend_name(const SbjSession *session, char *buf, size_t len);

/* Re-scans devices; then read each with sbj_session_device_info. */
uint32_t sbj_session_device_count(const SbjSession *session);
SbjStatus sbj_session_device_info(const SbjSession *session, uint32_t index,
                                  SbjDeviceInfo *out);

/* ---- Show ------------------------------------------------------------- */

/* The show document as JSON. Free with sbj_string_free. */
char *sbj_session_show_json(const SbjSession *session);

/* {"bank", "learning": "cue:N|stop_all|next_bank|prev_bank"|null,
 *  "last_control": string|null, "midi_inputs": [string]|null (null = MIDI off),
 *  "osc_port": number|null, "dirty", "path",
 *  "outputs": [{"name","status":"running|on_backup|offline",
 *  "device_id","device_name","sample_rate","channels","xruns","error"}],
 *  "problems": [{"slot","message"}], "notices": [string]}.
 * Notices are delivered once. Free with sbj_string_free. */
char *sbj_session_status_json(const SbjSession *session);

SbjStatus sbj_session_new_show(const SbjSession *session);
/* SLOW: decodes all audio. Current show is untouched on failure. */
SbjStatus sbj_session_open_show(const SbjSession *session, const char *path);
/* path NULL = the show's current file. Writes atomically. */
SbjStatus sbj_session_save_show(const SbjSession *session, const char *path);
/* {"name", "slots" (pads per bank), "columns", "banks": [names]}.
 * Cue slots are bank * slots + pad; resizing keeps each cue's bank/pad. */
SbjStatus sbj_session_set_show_settings_json(const SbjSession *session, const char *json);
/* The visible bank (0-based): hotkeys and OSC pad numbers refer to it. */
SbjStatus sbj_session_set_bank(const SbjSession *session, uint32_t bank);
/* JSON array of outputs: [{"name", "device": {"id","name"}?, "backup": {...}?}] */
SbjStatus sbj_session_set_outputs_json(const SbjSession *session, const char *json);

/* ---- External control ------------------------------------------------ */

/* Listen to every MIDI input (true; call again to rescan) or stop. */
SbjStatus sbj_session_enable_midi(const SbjSession *session, bool enabled);
/* Listen for OSC on a UDP port; 0 stops. See sbj-session/src/control.rs
 * for the address space (/sbj/fire/<pad>, /sbj/stopall, ...). */
SbjStatus sbj_session_set_osc_port(const SbjSession *session, uint16_t port);
/* Assign the next MIDI message to target: "cue:<slot>", "stop_all",
 * "next_bank", "prev_bank". NULL cancels. */
SbjStatus sbj_session_learn_midi(const SbjSession *session, const char *target);
SbjStatus sbj_session_clear_midi(const SbjSession *session, const char *target);

/* Pre-game check: {"ready", "memory_mb", "checks": [{"level":
 * "ok|info|warning|error", "message"}]}. Free with sbj_string_free. */
char *sbj_session_preflight_json(const SbjSession *session);

/* ---- Cues ------------------------------------------------------------- */

/* SLOW if new audio: adds/replaces the cue in json's "slot". */
SbjStatus sbj_session_set_cue_json(const SbjSession *session, const char *json);
SbjStatus sbj_session_remove_cue(const SbjSession *session, uint32_t slot);
/* Swaps with whatever is in `to`. */
SbjStatus sbj_session_move_cue(const SbjSession *session, uint32_t slot, uint32_t to);

/* SLOW: decodes a file for display before use. duration_out may be NULL. */
SbjStatus sbj_session_load_sound(const SbjSession *session, const char *file,
                                 double *duration_out);
/* Fills out[2*buckets] with (min, max) pairs for a loaded file. */
SbjStatus sbj_session_waveform(const SbjSession *session, const char *file,
                               uint32_t buckets, float *out);

/* ---- Playback --------------------------------------------------------- */

/* Fires a pad as pressing it does: starts it; if playing, releases a vamp
 * or applies its retrigger setting. Handles exclusive groups and ducking.
 * Empty pad: no-op. */
SbjStatus sbj_session_fire_cue(const SbjSession *session, uint32_t slot);
SbjStatus sbj_session_play_cue_from(const SbjSession *session, uint32_t slot, double seconds);
SbjStatus sbj_session_stop_cue(const SbjSession *session, uint32_t slot, double fade_seconds);
SbjStatus sbj_session_stop_all(const SbjSession *session, double fade_seconds);
SbjStatus sbj_session_set_master_gain(const SbjSession *session, float gain);

/* States of `count` pads from `first`, in one call. */
SbjStatus sbj_session_cue_states(const SbjSession *session, uint32_t first, uint32_t count,
                                 SbjCueState *out);

#ifdef __cplusplus
}
#endif

#endif /* SPORTSBALL_JUKEBOX_H */
