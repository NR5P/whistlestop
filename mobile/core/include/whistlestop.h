/* Whistlestop's engine, for the iPhone app (Swift imports this through the
 * bridging header). Every string is UTF-8 JSON; free the ones returned with
 * ws_free(). See mobile/core/src/lib.rs. */
#ifndef WHISTLESTOP_H
#define WHISTLESTOP_H
#include <stdint.h>

int32_t  ws_start(const char *dir);      /* 0 ok, -1 failed */
uint64_t ws_changes(void);               /* moves whenever the state changes */
char    *ws_state(void);                 /* engine State */
char    *ws_rows(void);                  /* new waterfall rows, [[dB]] */
int32_t  ws_send(const char *cmd_json);  /* one engine Cmd */
char    *ws_config(void);                /* settings */
int32_t  ws_set_config(const char *json);/* save and apply */
void     ws_stop(void);
int32_t  ws_mercury_embedded(void);    /* 1: Mercury runs inside the app */
void     ws_free(char *s);

#endif
