// Deterministic ABI fixture, not a speech recognizer. No audio leaves this process.
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

static int creates, destroys, decodes, alive, peak, mode;
void fixture_reset(int next_mode) {
    creates = destroys = decodes = alive = peak = 0;
    mode = next_mode;
}
int fixture_count(int which) {
    int counts[] = {creates, destroys, decodes, alive, peak};
    return counts[which];
}
uint32_t minutes_sherpa_abi_version(void) { return 2; }
void *minutes_sherpa_create(const char *dir, char *error, size_t len) {
    (void)dir;
    creates++;
    if (mode == 1) {
        snprintf(error, len, "fixture model failure");
        return NULL;
    }
    alive++;
    if (alive > peak) peak = alive;
    return malloc(1);
}
char *minutes_sherpa_transcribe(void *handle, uint32_t rate, const float *samples, size_t count) {
    (void)handle;
    if (rate != 16000 || count == 0) return NULL;
    decodes++;
    if (mode == 2 && decodes == 2) return NULL;
    if (mode == 3 && decodes == 2) return strdup("");
    float max = 0;
    for (size_t i = 0; i < count; i++) if (samples[i] > max) max = samples[i];
    if (max < 0.4f) return strdup("Bonjour les amis de Montréal.");
    if (max < 0.65f) return strdup("Zażółć gęślą jaźń podczas spotkania.");
    return strdup("We will review the project tomorrow.");
}
void minutes_sherpa_free_string(char *text) { free(text); }
void minutes_sherpa_destroy(void *handle) { destroys++; alive--; free(handle); }
void *minutes_sherpa_stream_create(const char *dir, char *error, size_t len) {
    (void)dir; (void)error; (void)len; return NULL;
}
char *minutes_sherpa_stream_feed(void *h, uint32_t r, const float *s, size_t n) {
    (void)h; (void)r; (void)s; (void)n; return NULL;
}
char *minutes_sherpa_stream_finish(void *h) { (void)h; return NULL; }
bool minutes_sherpa_stream_reset(void *h) { (void)h; return false; }
void minutes_sherpa_stream_destroy(void *h) { (void)h; }
