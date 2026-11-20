#define PY_SSIZE_T_CLEAN
#include <Python.h>

static PyObject *hello(PyObject *self, PyObject *args) {
  return PyUnicode_FromString("hello from crucible");
}

static PyMethodDef methods[] = {
    {"hello", hello, METH_NOARGS, NULL},
    {NULL, NULL, 0, NULL},
};

static struct PyModuleDef module = {PyModuleDef_HEAD_INIT, "greet", NULL, -1, methods};

PyMODINIT_FUNC PyInit_greet(void) { return PyModule_Create(&module); }
