""" Hoon rules for Bazel. """

load("//:rules_hoon/hoon.bzl", _hoon_jam = "hoon_jam", _hoon_library = "hoon_library")

hoon_library = _hoon_library
hoon_jam = _hoon_jam
