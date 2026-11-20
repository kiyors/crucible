# Compiles greet.c the way setuptools' build_ext does, from the compiler and
# flags sysconfig reports, so a sysconfig that still names the upstream build
# machine fails here.
import shlex
import subprocess
import sys
import sysconfig

var = sysconfig.get_config_var
out = "greet" + var("EXT_SUFFIX")
subprocess.run(
    shlex.split(var("CC"))
    + ["-fPIC", "-I" + var("INCLUDEPY"), "-c", sys.argv[1], "-o", "greet.o"],
    check=True,
)
subprocess.run(shlex.split(var("LDSHARED")) + ["greet.o", "-o", out], check=True)
