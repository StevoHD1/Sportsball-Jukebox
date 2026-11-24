/* Compiles against the public header and links the static library, so CI
 * catches drift between the header and the Rust exports. Works with or
 * without an audio device (CI machines usually have none). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "sportsball_jukebox.h"

#define CHECK(cond)                                                     \
    do {                                                                \
        if (!(cond)) {                                                  \
            char msg[256];                                              \
            sbj_last_error_message(msg, sizeof msg);                    \
            fprintf(stderr, "smoke test failed: %s (line %d): %s\n",    \
                    #cond, __LINE__, msg);                              \
            return 1;                                                   \
        }                                                               \
    } while (0)

static void put_u16(FILE *f, uint16_t v) { fputc(v & 0xff, f); fputc(v >> 8, f); }
static void put_u32(FILE *f, uint32_t v) { put_u16(f, v & 0xffff); put_u16(f, v >> 16); }

/* Writes a 0.1 s mono 16-bit 44.1 kHz WAV of a square wave. */
static int write_wav(const char *path) {
    FILE *f = fopen(path, "wb");
    if (!f) return 0;
    const uint32_t frames = 4410, data = frames * 2;
    fwrite("RIFF", 1, 4, f); put_u32(f, 36 + data);
    fwrite("WAVEfmt ", 1, 8, f); put_u32(f, 16);
    put_u16(f, 1); put_u16(f, 1); put_u32(f, 44100); put_u32(f, 88200);
    put_u16(f, 2); put_u16(f, 16);
    fwrite("data", 1, 4, f); put_u32(f, data);
    for (uint32_t i = 0; i < frames; i++) put_u16(f, (i / 50) % 2 ? 8000 : (uint16_t)-8000);
    fclose(f);
    return 1;
}

int main(void) {
    printf("sportsball jukebox engine %s\n", sbj_version());
    CHECK(sbj_abi_check(sizeof(SbjDeviceInfo), sizeof(SbjOutputInfo), sizeof(SbjCueState)));

    /* Sounds */
    const char *path = "sbj_smoke_test.wav";
    CHECK(write_wav(path));
    SbjStatus status = SBJ_STATUS_INTERNAL_ERROR;
    SbjSound *sound = sbj_sound_load(path, 48000, &status);
    remove(path);
    CHECK(sound != NULL && status == SBJ_STATUS_OK);
    CHECK(sbj_sound_duration_seconds(sound) > 0.099 && sbj_sound_duration_seconds(sound) < 0.101);
    CHECK(sbj_sound_channels(sound) == 1);

    CHECK(sbj_sound_load("does-not-exist.wav", 0, &status) == NULL);
    CHECK(status == SBJ_STATUS_MEDIA_ERROR);
    char msg[256];
    CHECK(sbj_last_error_message(msg, sizeof msg) > 0);

    /* Session */
    SbjSession *session = sbj_session_create(16);
    CHECK(session != NULL);
    char backend[64];
    CHECK(sbj_session_backend_name(session, backend, sizeof backend) == SBJ_STATUS_OK);
    uint32_t devices = sbj_session_device_count(session);
    printf("backend %s, %u output device(s)\n", backend, devices);
    for (uint32_t i = 0; i < devices; i++) {
        SbjDeviceInfo info;
        CHECK(sbj_session_device_info(session, i, &info) == SBJ_STATUS_OK);
        printf("  %s%s (%u ch @ %u Hz)\n", info.name, info.is_default ? " [default]" : "",
               info.output_channels, info.sample_rate);
    }

    CHECK(sbj_session_set_cue_sound(session, 3, sound) == SBJ_STATUS_OK);
    sbj_sound_free(sound); /* the session keeps its own reference */
    CHECK(sbj_session_set_cue_loop(session, 3, true) == SBJ_STATUS_OK);
    CHECK(sbj_session_set_cue_gain(session, 3, 0.8f) == SBJ_STATUS_OK);

    SbjCueState cue;
    CHECK(sbj_session_cue_state(session, 3, &cue) == SBJ_STATUS_OK);
    CHECK(cue.assigned && cue.looping && !cue.playing && cue.gain == 0.8f);

    SbjOutputInfo out;
    if (devices > 0 && sbj_session_open_output(session, NULL) == SBJ_STATUS_OK) {
        CHECK(sbj_session_output_info(session, &out) == SBJ_STATUS_OK && out.is_open);
        printf("opened %u ch @ %u Hz\n", out.channels, out.sample_rate);
        CHECK(sbj_session_set_master_gain(session, 0.0f) == SBJ_STATUS_OK); /* stay quiet */
        CHECK(sbj_session_fire_cue(session, 3) == SBJ_STATUS_OK);
        CHECK(sbj_session_stop_all(session, 0.05) == SBJ_STATUS_OK);
        CHECK(sbj_session_poll(session) == SBJ_STATUS_OK);
        CHECK(sbj_session_close_output(session) == SBJ_STATUS_OK);
    } else {
        printf("no usable output device; skipping playback\n");
        CHECK(sbj_session_fire_cue(session, 3) == SBJ_STATUS_NO_OUTPUT);
    }
    CHECK(sbj_session_output_info(session, &out) == SBJ_STATUS_OK && !out.is_open);
    CHECK(sbj_session_fire_cue(session, 999) == SBJ_STATUS_INVALID_ARGUMENT);
    CHECK(sbj_session_fire_cue(NULL, 0) == SBJ_STATUS_NULL_POINTER);
    CHECK(sbj_session_clear_cue(session, 3) == SBJ_STATUS_OK);

    sbj_session_destroy(session);
    printf("ok\n");
    return 0;
}
