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
    CHECK(sbj_abi_check(sizeof(SbjDeviceInfo), sizeof(SbjCueState)));

    SbjSession *s = sbj_session_create();
    CHECK(s != NULL);
    char backend[64];
    CHECK(sbj_session_backend_name(s, backend, sizeof backend) == SBJ_STATUS_OK);
    uint32_t devices = sbj_session_device_count(s);
    printf("backend %s, %u output device(s)\n", backend, devices);
    for (uint32_t i = 0; i < devices; i++) {
        SbjDeviceInfo info;
        CHECK(sbj_session_device_info(s, i, &info) == SBJ_STATUS_OK);
        printf("  %s%s (%u ch @ %u Hz)\n", info.name, info.is_default ? " [default]" : "",
               info.output_channels, info.sample_rate);
    }
    CHECK(sbj_session_open_outputs(s) == SBJ_STATUS_OK);
    CHECK(sbj_session_set_master_gain(s, 0.0f) == SBJ_STATUS_OK); /* stay quiet */

    /* Audio + cue */
    const char *wav = "sbj_smoke_test.wav";
    CHECK(write_wav(wav));
    double duration = 0;
    CHECK(sbj_session_load_sound(s, wav, &duration) == SBJ_STATUS_OK);
    CHECK(duration > 0.099 && duration < 0.101);
    float peaks[2 * 8];
    CHECK(sbj_session_waveform(s, wav, 8, peaks) == SBJ_STATUS_OK);
    CHECK(peaks[0] < -0.2f && peaks[1] > 0.2f);

    char cue[512];
    snprintf(cue, sizeof cue,
             "{\"slot\":3,\"title\":\"Square\",\"hotkey\":\"q\",\"loop\":true,"
             "\"clips\":[{\"file\":\"%s\"}],"
             "\"markers\":[{\"id\":1,\"name\":\"half\",\"time\":0.05,"
             "\"action\":{\"kind\":\"fade_out\",\"seconds\":0.02}}]}",
             wav);
    CHECK(sbj_session_set_cue_json(s, cue) == SBJ_STATUS_OK);

    SbjCueState states[4];
    CHECK(sbj_session_cue_states(s, 0, 4, states) == SBJ_STATUS_OK);
    CHECK(states[3].assigned && states[3].looping && !states[3].playing);

    if (states[3].ready) {
        CHECK(sbj_session_fire_cue(s, 3) == SBJ_STATUS_OK);
        CHECK(sbj_session_play_cue_from(s, 3, 0.02) == SBJ_STATUS_OK);
        CHECK(sbj_session_stop_cue(s, 3, 0.01) == SBJ_STATUS_OK);
        CHECK(sbj_session_stop_all(s, 0.05) == SBJ_STATUS_OK);
        printf("played through a real device\n");
    } else {
        printf("no usable output device; skipping playback\n");
        CHECK(sbj_session_fire_cue(s, 3) == SBJ_STATUS_NO_OUTPUT);
    }
    CHECK(sbj_session_poll(s) == SBJ_STATUS_OK);

    /* Show documents */
    char *json = sbj_session_show_json(s);
    CHECK(json != NULL && strstr(json, "\"Square\"") != NULL);
    sbj_string_free(json);
    char *status = sbj_session_status_json(s);
    CHECK(status != NULL && strstr(status, "\"dirty\":true") != NULL);
    printf("status: %s\n", status);
    sbj_string_free(status);

    const char *show_path = "sbj_smoke_test.sbjshow";
    CHECK(sbj_session_set_show_settings_json(
              s, "{\"name\":\"Smoke\",\"slots\":12,\"columns\":6,"
                 "\"banks\":[\"Pregame\",\"Game\"]}") == SBJ_STATUS_OK);
    CHECK(sbj_session_set_bank(s, 1) == SBJ_STATUS_OK);
    CHECK(sbj_session_set_bank(s, 2) == SBJ_STATUS_SHOW_ERROR);
    CHECK(sbj_session_learn_midi(s, "cue:3") == SBJ_STATUS_OK);
    CHECK(sbj_session_learn_midi(s, NULL) == SBJ_STATUS_OK);
    CHECK(sbj_session_learn_midi(s, "bogus") == SBJ_STATUS_INVALID_ARGUMENT);
    CHECK(sbj_session_clear_midi(s, "stop_all") == SBJ_STATUS_OK);
    CHECK(sbj_session_enable_midi(s, true) == SBJ_STATUS_OK);
    CHECK(sbj_session_set_osc_port(s, 0) == SBJ_STATUS_OK);
    char *preflight = sbj_session_preflight_json(s);
    CHECK(preflight != NULL && strstr(preflight, "\"checks\"") != NULL);
    printf("preflight: %s\n", preflight);
    sbj_string_free(preflight);
    CHECK(sbj_session_enable_midi(s, false) == SBJ_STATUS_OK);
    CHECK(sbj_session_save_show(s, show_path) == SBJ_STATUS_OK);
    CHECK(sbj_session_new_show(s) == SBJ_STATUS_OK);
    CHECK(sbj_session_cue_states(s, 3, 1, states) == SBJ_STATUS_OK && !states[0].assigned);
    CHECK(sbj_session_open_show(s, show_path) == SBJ_STATUS_OK);
    CHECK(sbj_session_cue_states(s, 3, 1, states) == SBJ_STATUS_OK && states[0].assigned);
    CHECK(sbj_session_skip_cue(s, 3) == SBJ_STATUS_OK || sbj_session_skip_cue(s, 3) == SBJ_STATUS_NO_OUTPUT);
    CHECK(sbj_session_move_cue(s, 3, 4) == SBJ_STATUS_OK);
    CHECK(sbj_session_remove_cue(s, 4) == SBJ_STATUS_OK);
    CHECK(sbj_session_set_outputs_json(s, "[{\"name\":\"Main\"},{\"name\":\"Lobby\"}]") ==
          SBJ_STATUS_OK);
    remove(show_path);
    remove(wav);

    CHECK(sbj_session_open_show(s, "missing.sbjshow") == SBJ_STATUS_SHOW_ERROR);
    CHECK(sbj_session_fire_cue(s, 999) == SBJ_STATUS_INVALID_ARGUMENT);
    CHECK(sbj_session_fire_cue(NULL, 0) == SBJ_STATUS_NULL_POINTER);

    sbj_session_destroy(s);
    printf("ok\n");
    return 0;
}
