//! `@declare`: a declaration says what exists, and generates nothing

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use duka_backend::builtin::require::{self, LoadedModule};
use duka_backend::value::{DukaProto, RuntimeValue};
use duka_lib::harness::{run_last, to_ir};
use duka_lib::module::from_source;
use duka_shared::config::DukaConfig;
use duka_shared::types::SourceName;

#[test]
fn a_declared_function_is_a_note_and_not_a_function() {
    let r = run_last(
        r#"
@declare function strlen(s: string): int
return strlen == nil
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Bool(true));
}

#[test]
fn a_declared_value_is_not_assigned() {
    let r = run_last(
        r#"
@declare global HOSTNAME: string
@declare @const MAX: int
return HOSTNAME == nil and MAX == nil
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Bool(true));
}

#[test]
fn a_declared_object_is_not_built() {
    let r = run_last(
        r#"
@declare object Point
    x: int
    y: int
end
return Point == nil
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Bool(true));
}

#[test]
fn a_declaration_keeps_the_program_around_it() {
    let r = run_last(
        r#"
@declare function host_random(): int
local total = 0
for i = 1, 4 do total = total + i end
return total
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(10));
}

#[test]
fn a_declaration_inside_a_function_body_generates_nothing_either() {
    let r = run_last(
        r#"
local function outer()
    @declare function host_random(): int
    return 7
end
return outer()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(7));
}

#[test]
fn a_declared_function_takes_no_room_in_the_generated_code() {
    // The whole chunk is the declaration, so the only thing left to run is the
    // `return` every chunk ends with: nothing was built, and in particular no
    // closure was made for the function it describes.
    let ir = to_ir("@declare function strlen(s: string): int\n").unwrap();
    assert!(ir.nesteds.is_empty(), "{:#?}", ir.nesteds);
    assert_eq!(ir.instructions.len(), 1, "{:#?}", ir.instructions);
}

#[test]
fn a_value_written_to_a_declaration_is_judged_by_what_it_declares() {
    let err = run_last("@declare global FLAG: bool\nFLAG = 1\n")
        .expect_err("an int is not a bool, whatever the declaration is");
    assert!(err.to_string().contains("bool"), "{err}");
}

#[test]
fn a_declaration_with_a_value_is_refused() {
    let err = run_last("@declare global FLAG: bool = true\n").expect_err("nothing to build");
    assert!(err.to_string().contains("@declare"), "{err}");
}

#[test]
fn every_declaration_keeps_its_own_attributes() {
    // The `@` on the next line starts that line. Reading it as the suffix of the
    // variable above would move the attributes onto the wrong name, and neither
    // line would be an error -- which is what used to happen.
    let r = run_last(
        r#"
@declare global FLAG: bool
@declare @const LIMIT: int
FLAG = 1
return LIMIT
"#,
    )
    .expect_err("FLAG is declared bool and is given an int");
    assert!(r.to_string().contains("bool"), "{r}");
}

#[test]
fn a_suffix_attribute_still_belongs_to_the_variable_it_follows() {
    let ir = to_ir("@for_all\nlocal a @only_a, b = 1, 2\nreturn b\n").unwrap();
    assert!(!ir.instructions.is_empty());
}

// The part that crosses a `require`. A declaration is only interesting once it
// is somebody else's business, so these read a module that declares from a
// program that does not.
//
// Two ways in, because the compiler and the runtime reach a module by different
// means: the compiler walks the files, the runtime asks a loader.

static DIRS: AtomicUsize = AtomicUsize::new(0);
static SERIAL: Mutex<()> = Mutex::new(());

/// Take the lock even when a previous test panicked under it, so one failure
/// does not take the rest of the section with it
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Compile `main`, which reaches `./host` through a `require`
fn compile_requiring(main: &str, module: &str) -> Result<DukaProto, String> {
    let dir = std::env::temp_dir().join(format!(
        "duka_decl_{}_{}",
        std::process::id(),
        DIRS.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(dir.join("modules")).unwrap();
    std::fs::write(dir.join("modules").join("host.duka"), module).unwrap();
    let main_path = dir.join("main.duka");
    std::fs::write(&main_path, main).unwrap();
    let source = std::fs::read_to_string(&main_path).unwrap();
    from_source(
        &source,
        SourceName::File("main".into(), main_path.into()),
        DukaConfig::default(),
    )
    .map_err(|e| e.to_string())
}

/// Run `main` against a `host` the runtime loader serves from memory
fn run_requiring(main: &str, module: &str) -> Result<RuntimeValue, String> {
    let module = module.to_owned();
    require::reset();
    require::set_loader(move |name, _| {
        if name != "host" {
            return Err(format!("module '{name}' not registered"));
        }
        let proto = from_source(&module, SourceName::Unnamed, DukaConfig::default())
            .map_err(|e| format!("{e}"))?;
        Ok(LoadedModule::Executable { proto, path: None })
    });
    let r = run_last(main).map_err(|e| e.to_string());
    require::reset();
    r
}

#[test]
fn a_class_crosses_a_require_as_itself() {
    let _guard = serial();
    let module = "export object Point\n\
                  \x20   x: int\n\
                  \x20   function :init(v) self.x = v end\n\
                  \x20   function :len() return self.x end\n\
                  end\n";
    // the importer gets the class, and the class is the one the module declared
    let r = run_requiring(
        "local m = require(\"host\")\nreturn m.Point.new(7):len()\n",
        module,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(7), "and it runs");

    compile_requiring(
        "local m = require(\"host\")\n\
         local p = m.Point.new(1)\n\
         local n: int = p.x\n\
         local bad: string = p.x\nreturn n\n",
        module,
    )
    .expect_err("a field of a class from another module keeps its own type");
}

#[test]
fn a_class_in_a_type_position_is_the_class_rather_than_any() {
    let _guard = serial();
    let module = "export object Point\n    x: int\nend\n";
    // both spellings hand back the same class, so an instance is assignable to it
    compile_requiring(
        "local m = require(\"host\")\n\
         local p: RequireType(\"host\").Point = m.Point.new(1)\n\
         local q: int = p.x\nreturn q\n",
        module,
    )
    .expect("the class crossed both ways and is the same class");
    let err = compile_requiring(
        "local p: RequireType(\"host\").Point = 1\nreturn 1\n",
        module,
    )
    .expect_err("a number is not a class");
    assert!(err.contains("Point"), "{err}");
    // and it is not `any` either: `any` would have accepted the number above
    compile_requiring(
        "local m = require(\"host\")\nlocal n: string = m.Point\nreturn n\n",
        module,
    )
    .expect_err("`m.Point` is a class, not a string");
}

#[test]
fn a_class_of_the_same_name_in_two_modules_is_two_classes() {
    let _guard = serial();
    // both files call it `Point`, and both number it `0` in their own table of
    // classes, which is the whole reason a class says which module declared it
    let dir = std::env::temp_dir().join(format!(
        "duka_decl_two_{}_{}",
        std::process::id(),
        DIRS.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(dir.join("modules")).unwrap();
    std::fs::write(
        dir.join("modules").join("a.duka"),
        "export object Point\n    x: int\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("modules").join("b.duka"),
        "export object Point\n    y: int\nend\n",
    )
    .unwrap();
    let main_path = dir.join("main.duka");
    std::fs::write(
        &main_path,
        "local a = require(\"a\")\n\
         local b = require(\"b\")\n\
         local p: a.Point = b.Point.new()\nreturn p\n",
    )
    .unwrap();
    let source = std::fs::read_to_string(&main_path).unwrap();
    let err = from_source(
        &source,
        SourceName::File("main".into(), main_path.into()),
        DukaConfig::default(),
    )
    .expect_err("`b.Point` is not an `a.Point`");
    assert!(err.to_string().contains("Type"), "{err}");
}

#[test]
fn a_declaration_can_be_written_in_front_of_the_name() {
    // `@declare x: int` and `x: int @declare` are one statement; the one in
    // front is the one a declaration is normally written.
    let r = run_last(
        r#"
local function f()
    local @declare y: int
    return y == nil
end
return f()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Bool(true));
}

#[test]
fn a_declaration_in_front_still_takes_no_value() {
    let err = compile_requiring("local @declare y: int = 1\nreturn 1\n", "").unwrap_err();
    assert!(err.contains("takes no value"), "{err}");
}

const DECLARES_AND_BUILDS: &str =
    "@declare export function strlen(s: string): int\nexport function real() return 1 end";

#[test]
fn a_declared_export_is_not_in_the_module_the_importer_gets() {
    let _guard = serial();
    let r = run_requiring(
        r#"
local host = require("host")
return host.strlen == nil
"#,
        DECLARES_AND_BUILDS,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Bool(true), "the promise is not a value");
}

#[test]
fn a_built_export_next_to_a_declared_one_is_still_built() {
    let _guard = serial();
    let r = run_requiring(
        r#"
local host = require("host")
return host.real()
"#,
        DECLARES_AND_BUILDS,
    )
    .unwrap();
    assert_eq!(
        r,
        RuntimeValue::Int(1),
        "the export table is still returned"
    );
}

#[test]
fn a_module_of_nothing_but_declarations_still_hands_back_a_table() {
    let _guard = serial();
    let r = run_requiring(
        r#"
local host = require("host")
return host == nil
"#,
        "@declare export function strlen(s: string): int",
    )
    .unwrap();
    assert_eq!(
        r,
        RuntimeValue::Bool(false),
        "a module hands back a table whatever it exports"
    );
}

#[test]
fn a_declared_export_is_callable_with_what_it_promised() {
    let _guard = serial();
    compile_requiring(
        "local host = require(\"host\")\nlocal n: int = host.strlen(\"s\")\nreturn n\n",
        DECLARES_AND_BUILDS,
    )
    .expect("the signature crossed, so the call is judged against it");
}

#[test]
fn the_parameter_list_of_a_declared_export_crosses_too() {
    let _guard = serial();
    let err = compile_requiring(
        "local host = require(\"host\")\nreturn host.strlen()\n",
        DECLARES_AND_BUILDS,
    )
    .expect_err("it takes one argument");
    assert!(err.contains("arguments"), "{err}");
}

#[test]
fn the_return_of_a_declared_export_crosses_too() {
    let _guard = serial();
    let err = compile_requiring(
        "local host = require(\"host\")\nlocal s: string = host.strlen(\"s\")\nreturn s\n",
        DECLARES_AND_BUILDS,
    )
    .expect_err("an int is not a string");
    assert!(err.contains("Type"), "{err}");
}

#[test]
fn a_declared_value_reaches_the_importer_as_the_type_it_was_given() {
    let _guard = serial();
    compile_requiring(
        "local host = require(\"host\")\nlocal s: string = host.HOSTNAME\nreturn s\n",
        "@declare export global HOSTNAME: string",
    )
    .expect("the annotation crossed with it");
    let err = compile_requiring(
        "local host = require(\"host\")\nlocal n: int = host.HOSTNAME\nreturn n\n",
        "@declare export global HOSTNAME: string",
    )
    .expect_err("a string is not an int");
    assert!(err.contains("Type"), "{err}");
}

#[test]
fn a_require_type_reaches_a_declaration_of_another_file() {
    let _guard = serial();
    // The annotation is a function and a nil is not one, so this only compiles
    // if what crossed the boundary was the shape rather than `any`.
    let err = compile_requiring(
        "local g: RequireType(\"host\").strlen = nil\nreturn 1\n",
        DECLARES_AND_BUILDS,
    )
    .expect_err("a nil is not a function");
    assert!(err.contains("function"), "{err}");
}

#[test]
fn a_require_type_can_be_read_through() {
    let _guard = serial();
    // `RequireType("m").A.B` nests: the base of every dot is rebuilt as a path,
    // and the shape of that nesting is only known to the evaluator. Without a
    // provider to resolve the module with, this was an error rather than a type.
    compile_requiring(
        "local b: RequireType(\"host\").Outer.inner.deep = true\n\
         local top: int = 1\nreturn top\n",
        "export type Outer = { inner: { deep: bool }, top: int }",
    )
    .expect("a member read off a member read off a module");
    let err = compile_requiring(
        "local b: RequireType(\"host\").Outer.inner.deep = 1\nreturn 1\n",
        "export type Outer = { inner: { deep: bool }, top: int }",
    )
    .expect_err("a number is not a bool");
    assert!(err.contains("Type"), "{err}");
}

#[test]
fn a_require_type_of_an_ordinary_export_is_still_any() {
    let _guard = serial();
    // The same reading without `@declare`: what an ordinary export means in a
    // type position is the type it exports, and it exports none.
    compile_requiring(
        "local g: RequireType(\"host\").strlen = nil\nreturn 1\n",
        "export function strlen(s) return 1 end",
    )
    .expect("anything else answers as `any`");
}
