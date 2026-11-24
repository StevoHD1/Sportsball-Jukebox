/* Compiles against the public header and links the static library, so CI
 * catches any drift between the header and the Rust exports. */
#include <stdio.h>
#include <stdlib.h>
#include "sportsball_jukebox.h"

#define CHECK(cond)                                                    \
    do {                                                               \
        if (!(cond)) {                                                 \
            fprintf(stderr, "smoke test failed: %s (line %d)\n", #cond, \
                    __LINE__);                                         \
            return 1;                                                  \
        }                                                              \
    } while (0)

int main(void) {
    printf("sportsball jukebox engine %s\n", sbj_version());

    SbjEngine *engine = sbj_engine_create(48000, 2);
    CHECK(engine != NULL);
    CHECK(sbj_engine_load_test_tone(engine, 440.0f, 0.5f) == SBJ_STATUS_OK);
    CHECK(sbj_engine_play(engine) == SBJ_STATUS_OK);

    float out[256 * 2];
    int heard = 0;
    for (int block = 0; block < 10; block++) {
        CHECK(sbj_engine_render(engine, out, 256) == SBJ_STATUS_OK);
        for (int i = 0; i < 256 * 2; i++) {
            if (out[i] != 0.0f) heard = 1;
        }
    }
    CHECK(heard);
    CHECK(sbj_engine_position(engine) == 2560);

    CHECK(sbj_engine_seek(engine, 100) == SBJ_STATUS_OK);
    CHECK(sbj_engine_pause(engine) == SBJ_STATUS_OK);
    CHECK(sbj_engine_render(engine, out, 256) == SBJ_STATUS_OK);
    CHECK(sbj_engine_position(engine) == 100);
    CHECK(sbj_engine_stop(engine) == SBJ_STATUS_OK);
    CHECK(sbj_engine_play(NULL) == SBJ_STATUS_NULL_POINTER);

    sbj_engine_destroy(engine);
    printf("ok\n");
    return 0;
}
