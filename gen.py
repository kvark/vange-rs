import os
import sys

print("\t// vangers-rs resources")
for root, subdirs, files in os.walk("res"):
    for file in files:
        path = os.path.join(root, file)
        print("\tcreate_file(\"" + path + "\", include_bytes!(\"../" + path + "\"));");

print("\n\t// vangers resouces")
for root, subdirs, files in os.walk("res_linux"):
    for file in files:
        path = os.path.join(root, file)
        if ("/video/" in path or ".git/" in path or
            "/sound/" in path or "/actint/" in path or
            "/iscreen/" in path or "/music/" in path or
            "/savegame/" in path or "/shader/" in path or
            ".wgsl" in path or "data.vot" in path or
            ".bnl" in path or ".lst" in path):
            continue
        print("\tcreate_file(\"" + path[len("res_linux/"):] + "\", include_bytes!(\"../" + path + "\"));");

