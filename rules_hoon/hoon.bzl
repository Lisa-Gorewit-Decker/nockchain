""" Hoon rules for Bazel build system.
This file contains rules for compiling Hoon source files into JAM files
using the hoonc compiler. It provides a rule for compiling Hoon files
and a higher-level macro for creating libraries from Hoon files.
"""

def _hoon_jam_impl(ctx):
    """Implementation of the hoon_jam rule without extra copying."""

    # Print present working directory
    # print("DEBUG: Current working directory: " + str(ctx.configuration))
    # Output file
    output = ctx.outputs.out

    # Create only the temp directory for hoonc's internal use
    tmp_dir = ctx.actions.declare_directory("_tmp_{}".format(ctx.label.name))

    # print("DEBUG: Temp dir: " + tmp_dir.path)
    # Collect the main source file and all deps
    src = ctx.file.src
    deps = depset(
        direct = ctx.files.deps,
        transitive = [dep[DefaultInfo].files for dep in ctx.attr.deps],
    ).to_list()

    # print("DEBUG: Deps: " + str([f.path for f in ctx.files.deps]))
    # Create the command to build the JAM file
    cmd = []
    cmd.append("set -e")  # Exit immediately if any command fails

    # Create temp directory
    cmd.append("mkdir -p {}".format(tmp_dir.path))

    # Resolve the root of the Hoon source tree from the source path: hoon/
    # at the repository root, or nested one directory deep (<parent>/hoon).
    src_path = src.path
    src_dir_parts = src_path.split("/")

    if src_dir_parts[0] == "hoon":
        hoon_dir = "hoon"
    else:
        hoon_dir = "/".join(src_dir_parts[:2])

    # Run hoonc directly with the source path
    hoonc_args = ["--new"]
    if ctx.attr.arbitrary:
        hoonc_args.append("--arbitrary")

    # print("Hoon dir: " + hoon_dir)
    cmd.append("env -i HOME={0} XDG_DATA_HOME={0}/.local/share XDG_CONFIG_HOME={0}/.config TMPDIR={1} RUST_LOG=trace {2} {3} {4} {5}".format(
        hoon_dir,  # Use the original location instead of copying
        tmp_dir.path,
        ctx.executable._hoonc.path,
        " ".join(hoonc_args),
        src_path,  # Use original source path
        hoon_dir,  # Pass the hoon directory directly
    ))

    # Copy the output file
    cmd.append("mv out.jam {}".format(output.path))

    # Execute the command
    ctx.actions.run_shell(
        inputs = [src] + deps,
        outputs = [tmp_dir, output],
        tools = [ctx.executable._hoonc],
        command = "\n".join(cmd),
        progress_message = "Building JAM file from %s" % src.path,
        mnemonic = "HoonCompile",
        use_default_shell_env = False,
    )

    return [DefaultInfo(files = depset([output]))]

# Define the rule
hoon_jam = rule(
    implementation = _hoon_jam_impl,
    attrs = {
        "src": attr.label(
            allow_single_file = [".hoon"],
            mandatory = True,
            doc = "Main Hoon source file to compile",
        ),
        "deps": attr.label_list(
            allow_files = [".hoon", ".jam"],
            doc = "Hoon dependencies",
        ),
        "out": attr.output(mandatory = True),
        "arbitrary": attr.bool(
            default = False,
            doc = "Whether to use the --arbitrary flag",
        ),
        "output": attr.bool(
            default = False,
            doc = "Whether to use the --output flag",
        ),
        "_hoonc": attr.label(
            default = Label("//crates/hoonc:hoonc_bin"),
            executable = True,
            cfg = "exec",
        ),
    },
    doc = "Compiles a Hoon source file into a JAM file using Choo",
)

# Higher-level macro for common patterns
def hoon_library(
        name,
        src,
        deps = [],
        arbitrary = False,
        visibility = None):
    """Builds a JAM file from a Hoon source file.

    Args:
        name: Name of the target
        src: Main Hoon source file
        deps: Hoon dependencies (typically //hoon:all_hoon_files)
        arbitrary: Whether to use --arbitrary flag
        visibility: Target visibility
    """
    jam_name = name + ".jam"

    hoon_jam(
        name = name + "_compile",
        src = src,
        deps = deps,
        out = jam_name,
        arbitrary = arbitrary,
        visibility = ["//visibility:private"],
    )

    native.filegroup(
        name = name,
        srcs = [":" + name + "_compile"],
        visibility = visibility,
    )
