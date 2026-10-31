#include "my_application.h"

int main(int argc, char** argv) {
  // Flutter's GL queries and framebuffer operations synchronize Mesa's command
  // thread with the raster thread. Avoid that extra queue for this process,
  // while preserving an explicit driver setting supplied by the user.
  g_setenv("mesa_glthread", "false", FALSE);

  g_autoptr(MyApplication) app = my_application_new();
  return g_application_run(G_APPLICATION(app), argc, argv);
}
