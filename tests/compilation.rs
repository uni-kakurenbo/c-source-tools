use c_source_tools::Build;
use c_source_tools::{read_text, write_text};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "c-source-tools-gcc-{}-{timestamp} 日本語",
            std::process::id()
        ));

        Self(root)
    }

    fn put(&self, path: &str, text: &str) {
        write_text(&self.0.join(path), text).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // This guard owns a unique directory immediately inside the OS temp directory.
        assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn checked(command: &mut Command) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("Could not execute {command:?}: {error}"));
    assert!(
        output.status.success(),
        "{command:?}: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    output
}

#[test]
fn modular_and_bundled_sources_compile_and_preserve_template_output() {
    let fixture = Fixture::new();
    macro_rules! put {
        ($path:literal) => {
            fixture.put($path, include_str!(concat!("fixtures/modules/", $path)))
        };
    }
    put!("src/api.h");
    put!("src/left/message.asm");
    put!("src/left/unit.h");
    put!("src/left/unit.c");
    put!("src/right/message.asm");
    put!("src/right/unit.h");
    put!("src/right/unit.c");
    put!("src/main.c");
    fixture.put("src/profiles/fixture.c", "int fixture_profile;\n");

    let root = &fixture.0;
    let output = Build::new(root)
        .unwrap()
        .bundle(&entries("fixture"), Path::new("dist/fixture.c"))
        .unwrap();
    assert!(
        !read_text(Path::new(&output.bundle))
            .unwrap()
            .contains("#pragma once")
    );
    let generated = Path::new(&output.generated_root);
    let compiler = std::env::var_os("CC")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "gcc".into());
    let flags = ["-std=c11", "-Wall", "-Wextra", "-Wpedantic", "-Werror"];
    let mut objects = Vec::new();

    for source in &output.sources {
        let object = root.join(format!("objects/{source}.o"));
        fs::create_dir_all(object.parent().unwrap()).unwrap();
        let local_generated = generated.join(Path::new(source).parent().unwrap());

        checked(
            Command::new(&compiler)
                .current_dir(root)
                .args(flags)
                .arg("-Isrc")
                .arg(format!("-I{}", generated.join("src").display()))
                .arg(format!("-I{}", local_generated.display()))
                .arg("-c")
                .arg(source)
                .arg("-o")
                .arg(&object),
        );
        objects.push(object);
    }

    let extension = if cfg!(windows) { "exe" } else { "bin" };
    let modular = root.join(format!("modular.{extension}"));
    checked(
        Command::new(&compiler)
            .current_dir(root)
            .args(&objects)
            .arg("-o")
            .arg(&modular),
    );

    let standalone = root.join("isolated/program.c");
    write_text(&standalone, &read_text(Path::new(&output.bundle)).unwrap()).unwrap();
    let bundled = standalone.with_extension(extension);
    checked(
        Command::new(&compiler)
            .current_dir(root)
            .args(flags)
            .arg(&standalone)
            .arg("-o")
            .arg(&bundled),
    );

    for binary in [modular, bundled] {
        let result = checked(Command::new(&binary).current_dir(root));
        assert!(result.stderr.is_empty());
        assert_eq!(
            String::from_utf8(result.stdout)
                .unwrap()
                .replace("\r\n", "\n"),
            "value State 7 9\n%eax \"quoted\" path\\file 42 42 %s\nother 11\n"
        );
    }
}

#[test]
fn c23_private_declarations_are_isolated_between_modules() {
    let fixture = Fixture::new();
    fixture.put(
        "src/main.c",
        "#include \"left.h\"\n#include \"right.h\"\nint main(void) { return left() + right() != 42; }\n",
    );

    fixture.put("src/profiles/fixture.c", "int fixture_profile;\n");

    for (module, value) in [("left", 20), ("right", 22)] {
        fixture.put(
            &format!("src/{module}.h"),
            &format!("#pragma once\nint {module}(void);\n"),
        );

        fixture.put(
            &format!("src/{module}.c"),
            &format!(
                r#"#include "{module}.h"
#include <stdint.h>
#include <stdlib.h>

typedef enum Mode : uint64_t {{ MODE = {value} }} Mode;
constexpr uint64_t noreturn = MODE;
static_assert(sizeof(Mode) == sizeof(uint64_t));

[[noreturn]] static void fail(int) {{ abort(); }}

int {module}(void) {{
    if (noreturn != {value}) fail(0);
    return (int)noreturn;
}}
"#
            ),
        );
    }

    let root = &fixture.0;
    let output = Build::new(root)
        .unwrap()
        .bundle(&entries("fixture"), Path::new("dist/fixture.c"))
        .unwrap();

    let compiler = std::env::var_os("CC")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "gcc".into());
    let binary = root.join(if cfg!(windows) { "fixture.exe" } else { "fixture" });

    checked(
        Command::new(compiler)
            .current_dir(root)
            .args(["-std=c23", "-Wall", "-Wextra", "-Wpedantic", "-Werror"])
            .arg(output.bundle)
            .arg("-o")
            .arg(&binary),
    );

    checked(Command::new(binary).current_dir(root));
}

#[test]
fn templates_supply_their_standard_headers_in_modular_and_bundled_sources() {
    let fixture = Fixture::new();
    fixture.put("src/plain.asm", "@plain\nplain\n");
    fixture.put(
        "src/message.asm",
        "@show signed_value:int unsigned_value:uint length:size text:string\n{{signed_value}} {{unsigned_value}} {{length}} {{text}}\n",
    );
    // This declaration follows the template's stdio.h in the modular source.
    // Bundling must move that system header ahead of expanded project headers.
    fixture.put("src/output.h", "typedef FILE Output;\n");
    fixture.put(
        "src/main.c",
        concat!(
            "#include \"plain.inc\"\n#include \"message.inc\"\n#include \"output.h\"\n",
            "int main(void) {\n",
            "    Output *output = stdout;\n",
            "    asm_plain(output);\n",
            "    asm_show(output, -1, 2, 3, \"text\");\n",
            "    return 0;\n}\n",
        ),
    );
    fixture.put("src/profiles/fixture.c", "int fixture_profile;\n");
    let output = Build::new(&fixture.0)
        .unwrap()
        .bundle(&entries("fixture"), Path::new("dist/fixture.c"))
        .unwrap();
    let bundled = read_text(Path::new(&output.bundle)).unwrap();
    assert_eq!(bundled.matches("#include <stdio.h>").count(), 1);
    assert!(
        read_text(&Path::new(&output.generated_root).join("src/message.inc"))
            .unwrap()
            .starts_with("#include <stdio.h>\n")
    );

    let compiler = std::env::var_os("CC")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "gcc".into());
    let flags = ["-std=c11", "-Wall", "-Wextra", "-Wpedantic", "-Werror"];
    let extension = if cfg!(windows) { "exe" } else { "bin" };
    for (name, source) in [
        ("modular", fixture.0.join("src/main.c")),
        ("bundled", PathBuf::from(&output.bundle)),
    ] {
        let executable = fixture.0.join(format!("{name}.{extension}"));
        checked(
            Command::new(&compiler)
                .current_dir(&fixture.0)
                .args(flags)
                .arg(format!("-I{}/src", output.generated_root))
                .arg(source)
                .arg("-o")
                .arg(&executable),
        );
        let result = checked(&mut Command::new(executable));
        assert_eq!(
            String::from_utf8(result.stdout)
                .unwrap()
                .replace("\r\n", "\n"),
            "plain\n-1 2 3 text\n"
        );
    }
}

fn entries(name: &str) -> Vec<String> {
    vec!["src/main.c".into(), format!("src/profiles/{name}.c")]
}
