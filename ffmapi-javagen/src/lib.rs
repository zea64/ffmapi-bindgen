use ffmapi_bindgen_common::*;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path as OsPath;
use std::{error::Error, fs};
use syn::{Item, ItemFn, Type};

fn rust_type_to_value_layout(rust_arg: &RustFnArg) -> (&'static str, &'static str) {
	match rust_arg.kind {
		ArgKind::Primitive => {
			// Here we pass the actual `TypePath` to `primitive_match`
			if let Type::Path(type_path) = &rust_arg.ty {
				if let Some(java_primitive) = primitive_match(type_path) {
					match java_primitive {
						"f64" => ("double", "ValueLayout.JAVA_DOUBLE"),
						"f32" => ("float", "ValueLayout.JAVA_FLOAT"),
						"i64" | "isize" => ("long", "ValueLayout.JAVA_LONG"),
						"i32" => ("int", "ValueLayout.JAVA_INT"),
						"i16" => ("short", "ValueLayout.JAVA_SHORT"),
						"i8" => ("byte", "ValueLayout.JAVA_BYTE"),
						"u64" => ("long", "ValueLayout.JAVA_LONG"),
						"u32" => ("int", "ValueLayout.JAVA_INT"),
						"u16" => ("short", "ValueLayout.JAVA_SHORT"),
						"u8" => ("byte", "ValueLayout.JAVA_BYTE"),
						"bool" => ("boolean", "ValueLayout.JAVA_BOOLEAN"),
						"char" => ("char", "ValueLayout.JAVA_CHAR"),
						_ => ("MemorySegment", "ValueLayout.ADDRESS"),
					}
				} else {
					("MemorySegment", "ValueLayout.ADDRESS")
				}
			} else {
				("MemorySegment", "ValueLayout.ADDRESS")
			}
		}
		ArgKind::Address => ("MemorySegment", "ValueLayout.ADDRESS"),
		ArgKind::Boxed => ("MemorySegment", "ValueLayout.ADDRESS"),
	}
}

pub fn generate_java_code(fn_item: &ItemFn) -> Result<(), Box<dyn Error>> {
	let fn_name = fn_item.sig.ident.to_string();
	let class_name = capitalize_first_letter(&fn_name);

	let mut existing_types: HashSet<&str> = HashSet::new();

	// Extracting arguments
	let args = extract_args(fn_item).expect("Failed to extract function arguments");

	// Prepare definitions for the MethodHandle
	let definitions_content = format!("private static final MethodHandle {}Handle;\n", fn_name);
	let drop_definitions_content = format!(
		"private static final MethodHandle drop{}Handle;\n",
		class_name
	);

	let mut params = Vec::new();
	for rust_arg in args.iter() {
		params.push(rust_type_to_value_layout(rust_arg).1);
		existing_types.insert(rust_type_to_value_layout(rust_arg).1);
	}

	// // Prepare static initializer block with MethodHandle setup
	// let mut params = Vec::new();
	// for input in fn_item.sig.inputs.iter() {
	// 	if let FnArg::Typed(PatType { ty, .. }) = input {
	// 		if let Type::Path(type_path) = &**ty {
	// 			let param_type = type_path.path.segments.first().unwrap().ident.to_string();
	// 			params.push(rust_type_to_value_layout(&param_type).1);
	// 		}
	// 	}
	// }

	let return_type = extract_return_type(fn_item).expect("Failed to extract return type");

	let return_type_string = get_return_type_string(Option::from(&return_type))?;

	existing_types.insert(return_type_string.as_str());

	// Generating dummy classes
	generate_type_classes(existing_types)?;

	// let return_type = if let syn::ReturnType::Type(_, return_type) = &fn_item.sig.output {
	// 	if let Type::Path(type_path) = &**return_type {
	// 		rust_type_to_value_layout(&type_path.path.segments.first().unwrap().ident.to_string()).1
	// 	} else {
	// 		"ValueLayout.ADDRESS"
	// 	}
	// } else {
	// 	"FunctionDescriptor.ofVoid()"
	// };

	// Format the function descriptor with parameter layouts
	let param_layouts = if params.is_empty() {
		format!("FunctionDescriptor.of({})", return_type_string)
	} else {
		format!(
			"FunctionDescriptor.of({}, {})",
			return_type_string,
			params.join(", ")
		)
	};

	// Method handle implementation block
	let implementations_content = format!(
		"        {fn_name}Handle = Global.linker.downcallHandle(\n            Global.lib.find(\"{fn_name}\").orElseThrow(),\n            {param_layouts}\n        );\n",
		fn_name = fn_name,
		param_layouts = param_layouts
	);

	let drop_implementations_content = format!(
		"        drop{class_name}Handle = Global.linker.downcallHandle(\n            Global.lib.find(\"{fn_name}_drop\").orElseThrow(),\n            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)\n        );\n",
		class_name = class_name,
		fn_name = fn_name
	);

	// Prepare `make` method content
	let make_method_content = format!(
		r#"
    public static Ref<{class_name}> make{class_name}(int x) throws Throwable {{
        try (Arena arena = Arena.ofConfined()) {{
            MemorySegment inputSegment = arena.allocate(ValueLayout.JAVA_INT);
            inputSegment.set(ValueLayout.JAVA_INT, 0, x);

            MemorySegment resultSegment = (MemorySegment) {fn_name}Handle.invoke(inputSegment);
            {class_name} {lower_class_name} = new {class_name}(resultSegment);
            return new Ref<>(new RefCell<>({lower_class_name}));
        }}
    }}
    "#,
		class_name = class_name,
		fn_name = fn_name,
		lower_class_name = fn_name.to_lowercase()
	);

	// Prepare `drop` method content
	let drop_method_content = format!(
		r#"
    static void drop{class_name}({class_name} {lower_class_name}) {{
        try {{
            drop{class_name}Handle.invoke({lower_class_name}.getMemorySegment().address());
        }} catch (Throwable e) {{
            throw new RuntimeException("Failed to release {class_name} memory", e);
        }}
    }}
    "#,
		class_name = class_name,
		lower_class_name = fn_name.to_lowercase()
	);

	// Write to definitions.txt
	let mut definitions_file = OpenOptions::new()
		.create(true)
		.append(true)
		.open("./target/generated_code/definitions.txt")
		.expect("Failed to open definitions file");
	definitions_file
		.write_all(definitions_content.as_bytes())
		.expect("Failed to write definitions");
	definitions_file
		.write_all(drop_definitions_content.as_bytes())
		.expect("Failed to write drop definitions");

	// Write to implementations.txt
	let mut implementations_file = OpenOptions::new()
		.create(true)
		.append(true)
		.open("./target/generated_code/implementations.txt")
		.expect("Failed to open implementations file");
	implementations_file
		.write_all(implementations_content.as_bytes())
		.expect("Failed to write implementations");
	implementations_file
		.write_all(drop_implementations_content.as_bytes())
		.expect("Failed to write drop implementations");

	// Write to make_file.txt
	let mut make_file = OpenOptions::new()
		.create(true)
		.append(true)
		.open("./target/generated_code/make_file.txt")
		.expect("Failed to open make file");
	make_file
		.write_all(make_method_content.as_bytes())
		.expect("Failed to write make method");

	// Write to drop_file.txt
	let mut drop_file = OpenOptions::new()
		.create(true)
		.append(true)
		.open("./target/generated_code/drop_file.txt")
		.expect("Failed to open drop file");
	drop_file
		.write_all(drop_method_content.as_bytes())
		.expect("Failed to write drop method");

	Ok(())
}

// Overwrites previously generated files

pub fn generate_wrapper_class(fn_item: &ItemFn) -> Result<(), Box<dyn Error>> {
	let fn_name = fn_item.sig.ident.to_string();
	let class_name = capitalize_first_letter(&fn_name);

	let return_type_arg = extract_return_type(fn_item);

	// Determine the Java type and ValueLayout based on the Rust return type
	let (java_type, value_layout) = match return_type_arg? {
		Some(ref arg) => match &arg.kind {
			ArgKind::Primitive => rust_type_to_value_layout(arg),
			ArgKind::Boxed | ArgKind::Address => ("MemorySegment", "ValueLayout.ADDRESS"),
		},
		None => ("void", ""), // No return type (void)
	};

	let class_content = format!(
		r#"
import java.lang.foreign.MemorySegment;
import java.lang.foreign.ValueLayout;

public class {class_name} implements AutoCloseable {{
    private MemorySegment value;

    {class_name}(MemorySegment value) {{
        this.value = value;
    }}

    public {java_type} getValue() {{
        if (value == null) {{
            throw new IllegalStateException("MemorySegment has been released.");
        }}
        return value.get({value_layout}, 0);
    }}

    @Override
    public void close() {{
        if (this.value != null) {{
            RustFunctions.drop{class_name}(this);
            this.value = null;
        }}
    }}

    MemorySegment getMemorySegment() {{
        return this.value;
    }}
}}
"#,
		class_name = class_name,
		java_type = java_type,
		value_layout = value_layout
	);

	let output_file = format!("./target/generated_code/{}.java", class_name);
	let mut file = OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(true)
		.open(&output_file)
		.expect("Unable to open or create Java wrapper class file");

	file.write_all(class_content.as_bytes())
		.expect("Failed to write Java wrapper class content");

	Ok(())
}

// Generate dummy classes
pub fn generate_type_classes(types: HashSet<&str>) -> Result<(), Box<dyn Error>> {
	let mut make_file = OpenOptions::new()
		.create(true)
		.append(true)
		.open("./target/generated_code/DummyClasses.java")
		.expect("Failed to open make file");

	for &data_type in &types {
		make_file
			.write_all(generate_type_class_from_name(data_type).as_bytes())
			.expect("Failed to write make method");
	}
	Ok(())
}

// Format type string into dummy class string
pub fn generate_type_class_from_name(name: &str) -> &str {
	let class_content = format!(
		r#"
public class R{name} {{
	MemorySegment value;
}}

"#,
		name = name
	);
	class_content.as_str()
}

pub fn combine_files() -> Result<(), Box<dyn Error>> {
	let output_file = "./target/generated_code/RustFunctions.java";
	let class_header = r#"
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.lang.foreign.FunctionDescriptor;

public class RustFunctions {
"#;
	let class_footer = r#"
    } catch (Throwable e) {
        throw new RuntimeException("Failed to initialize Rust function handles", e);
    }
  }
}
"#;

	let mut output = OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(true)
		.open(output_file)?;
	output.write_all(class_header.as_bytes())?;

	let definitions_content = fs::read_to_string("./target/generated_code/definitions.txt")?;
	output.write_all(definitions_content.as_bytes())?;
	output.write_all(b"    static {\n        try {\n")?;

	let implementations_content =
		fs::read_to_string("./target/generated_code/implementations.txt")?;
	output.write_all(implementations_content.as_bytes())?;
	output.write_all(b"    } catch (Throwable e) {\n        throw new RuntimeException(\"Failed to initialize Rust function handles\", e);\n    }\n  }\n")?;

	let make_content = fs::read_to_string("./target/generated_code/make_file.txt")?;
	output.write_all(make_content.as_bytes())?;

	let drop_content = fs::read_to_string("./target/generated_code/drop_file.txt")?;
	output.write_all(drop_content.as_bytes())?;

	output.write_all(class_footer.as_bytes())?;

	fs::remove_file("./target/generated_code/definitions.txt")?;
	fs::remove_file("./target/generated_code/implementations.txt")?;
	fs::remove_file("./target/generated_code/make_file.txt")?;
	fs::remove_file("./target/generated_code/drop_file.txt")?;

	Ok(())
}

fn capitalize_first_letter(s: &str) -> String {
	let mut c = s.chars();
	match c.next() {
		None => String::new(),
		Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
	}
}

fn get_return_type_string(return_type_arg: Option<&RustFnArg>) -> Result<String, Box<dyn Error>> {
	match return_type_arg {
		Some(arg) => {
			match arg.kind {
				ArgKind::Primitive => {
					// Use `primitive_match` to get the primitive type as a string
					let type_path = match arg.ty {
						Type::Path(ref x) => x,
						_ => return Err("fixme".into()),
					};
					Ok(primitive_match(&type_path)
						.unwrap_or("MemorySegment")
						.into())
				}
				ArgKind::Boxed | ArgKind::Address => {
					// For boxed or address types, return a generic representation
					Ok("MemorySegment".to_string())
				}
			}
		}
		None => Ok("void".to_string()), // No return type (void)
	}
}

// Java code ends here

pub fn parse_file(file_path: &OsPath) -> Result<(), Box<dyn Error>> {
	println!("cargo::rerun-if-changed={}", file_path.to_str().unwrap());

	let content = fs::read_to_string(file_path)?;
	let ast = syn::parse_file(&content)?;

	for item in ast.items {
		match item {
			Item::Fn(f) => {
				if f.attrs
					.iter()
					.map(|attr| attr.meta.path())
					.any(|path| path.segments.first().unwrap().ident == "java_export")
				{
					println!("cargo::warning={:?}", f);
					generate_java_code(&f)?;
					generate_wrapper_class(&f)?;
					combine_files()?;
				}
			}
			Item::Mod(m) => {
				// Try "./${mod}.rs".
				let mod_file = format!("{}.rs", m.ident);
				let mut mod_path = file_path.to_path_buf();
				mod_path.pop();
				mod_path.push(mod_file);
				let _ = parse_file(mod_path.as_path());

				// Try "./${mod}/mod.rs".
				mod_path.pop();
				mod_path.push(m.ident.to_string());
				mod_path.push("mod.rs");
				let _ = parse_file(mod_path.as_path());
			}
			_ => (),
		}
	}

	Ok(())
}
