use core::error::Error;
use ffmapi_bindgen_common::*;
use std::{
	collections::HashSet,
	fmt::Write as _,
	fs,
	io::Write as _,
	path::{Path, PathBuf},
};

use syn::{parse_file, Ident, Item, ItemFn, ReturnType, Type};

use proc_macro2::Span;

pub fn generate_bindings(
	output_dir: &Path,
	input_file: &Path,
	libpath: &Path,
) -> Result<(), Box<dyn Error>> {
	let file = fs::read_to_string(input_file)?;
	let ast = parse_file(&file)?;

	let mut state = State::new(output_dir.to_path_buf());

	for item in ast.items {
		if let Item::Fn(f) = item {
			// Find functions with our macro.
			if f.attrs
				.iter()
				.map(|attr| attr.meta.path())
				.any(|path| path.segments.first().unwrap().ident == "java_export")
			{
				process_fn(f, &mut state)?;
			}
		}
	}

	finalize(state, libpath.to_str().unwrap())?;

	write_files(output_dir)?;

	Ok(())
}

#[derive(Debug, Default)]
struct State {
	types: HashSet<String>,
	methods: Vec<Method>,
	path: PathBuf,
}

impl State {
	fn new(path: PathBuf) -> Self {
		Self {
			types: HashSet::new(),
			methods: Vec::new(),
			path,
		}
	}
}

#[derive(Debug)]
struct Method {
	name: String,
	sig: (Option<RustFnArg>, Vec<RustFnArg>),
}

fn process_fn(f: ItemFn, state: &mut State) -> Result<(), Box<dyn Error>> {
	let args = extract_args(&f)?;
	let return_arg = match f.sig.output {
		ReturnType::Default => None,
		ReturnType::Type(_, ty) => Some(RustFnArg {
			ident: Ident::new("return_type", Span::call_site()),
			kind: ArgKind::from_type(&ty).unwrap(),
			ty: *ty,
		}),
	};

	for arg in args.iter().chain(&return_arg) {
		if arg.kind == ArgKind::Boxed {
			let ty_string = canoncalize_type(arg).unwrap();
			state.types.insert(ty_string);
		}
	}

	state.methods.push(Method {
		name: f.sig.ident.to_string(),
		sig: (return_arg, args),
	});

	Ok(())
}

fn canoncalize_type(arg: &RustFnArg) -> Option<String> {
	match arg.kind {
		ArgKind::Boxed => {
			let ty = match arg.ty {
				Type::Path(ref path) => path,
				_ => return None,
			};

			let last_segment = ty.path.segments.iter().last()?;
			Some(format!("R{}", last_segment.ident))
		}
		ArgKind::Address => match arg.ty {
			Type::Ptr(_) => Some("MemorySegment".to_owned()),
			Type::Reference(ref r) => {
				let ref_wrapper = if r.mutability.is_some() {
					"RefMut"
				} else {
					"Ref"
				};

				let inner = canoncalize_type(&RustFnArg {
					ident: Ident::new("placeholder", Span::call_site()),
					kind: ArgKind::from_type(&r.elem)?,
					ty: *r.elem.clone(),
				})?;

				Some(format!(
					"{}<{}>",
					ref_wrapper,
					maybe_primitive_to_boxed(&inner)
				))
			}
			_ => None,
		},
		ArgKind::Primitive => {
			if let Type::Path(ref p) = arg.ty {
				Some(primitive_to_java(primitive_match(p)?)?.to_owned())
			} else {
				None
			}
		}
	}
}

fn type_to_value_layout(arg: &RustFnArg) -> Result<String, Box<dyn Error>> {
	match arg.kind {
		ArgKind::Primitive => {
			if let Type::Path(ref p) = arg.ty {
				let primitive = primitive_to_java(primitive_match(p).unwrap()).unwrap();
				Ok(format!("ValueLayout.JAVA_{}", primitive.to_uppercase()))
			} else {
				unreachable!();
			}
		}
		ArgKind::Address | ArgKind::Boxed => Ok("ValueLayout.ADDRESS".to_owned()),
	}
}

// Box types like `int` to `Integer`
fn maybe_primitive_to_boxed(s: &str) -> &str {
	match s {
		"byte" => "Byte",
		"short" => "Short",
		"int" => "Integer",
		"long" => "Long",
		"float" => "Float",
		"double" => "Double",
		"boolean" => "Boolean",
		_ => s,
	}
}

// The real meat and potatos: write the files.
fn finalize(state: State, libname: &str) -> Result<(), Box<dyn Error>> {
	for ty in state.types {
		let mut path = state.path.clone();
		path.push(format!("{}.java", &ty));

		let mut file = fs::OpenOptions::new()
			.create(true)
			.truncate(true)
			.write(true)
			.open(&path)?;

		writeln!(
			&mut file,
			"\
import java.lang.foreign.MemorySegment;

public class {0} {{
	MemorySegment value;
	
	{0}(MemorySegment value) {{
		this.value = value;
	}}
}}",
			&ty
		)?;
	}

	let mut path = state.path;
	path.push("RustFns.java");
	let mut file = fs::OpenOptions::new()
		.create(true)
		.truncate(true)
		.write(true)
		.open(&path)?;

	write!(
		&mut file,
		r"import java.lang.invoke.MethodHandle;
import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;

public class RustFns {{
	static MethodHandle getHandle(Linker linker, SymbolLookup lookup, String symbol, FunctionDescriptor descriptor) throws Throwable {{
	var addr = lookup.find(symbol).get();
		return linker.downcallHandle(addr, descriptor);
	}}

"
	)?;

	// First pass: create static handles
	for method in state.methods.iter() {
		writeln!(&mut file, "\tstatic MethodHandle {}_handle;", &method.name)?;
	}

	// Second pass: set handles via linker
	write!(
		&mut file,
		r#"
		static {{
		try {{
			var linker = Linker.nativeLinker();
			var lib = SymbolLookup.libraryLookup("{}", Arena.global());
"#,
		libname
	)?;

	for method in state.methods.iter() {
		let mut need_leading_comma;
		let mut descriptor = if let Some(ref ret_type) = method.sig.0 {
			need_leading_comma = true;
			format!("FunctionDescriptor.of({}", type_to_value_layout(ret_type)?)
		} else {
			need_leading_comma = false;
			"FunctionDescriptor.ofVoid(".to_owned()
		};

		for arg in method.sig.1.iter() {
			if need_leading_comma {
				write!(&mut descriptor, ", ")?;
			}
			need_leading_comma = true;

			write!(&mut descriptor, "{}", type_to_value_layout(arg)?)?;
		}

		write!(&mut descriptor, ")")?;

		writeln!(
			&mut file,
			r#"			{0}_handle = getHandle(linker, lib, "{0}", {1});"#,
			&method.name, descriptor
		)?;
	}

	writeln!(
		&mut file,
		"		}} catch (Throwable e) {{
			e.printStackTrace();
		}}
	}}
"
	)?;

	// Final pass: wrapper methods
	for method in state.methods {
		if let Some(ref ret_type) = method.sig.0 {
			write!(
				&mut file,
				"	static {} {}(",
				canoncalize_type(ret_type).unwrap(),
				&method.name
			)?;
		} else {
			write!(&mut file, "	static void {}(", &method.name)?;
		}

		let mut need_leading_comma = false;
		for arg in method.sig.1.iter() {
			if need_leading_comma {
				write!(&mut file, ", ")?;
			}
			need_leading_comma = true;

			write!(
				&mut file,
				"{} {}",
				canoncalize_type(arg).unwrap(),
				arg.ident
			)?;
		}
		writeln!(&mut file, ") throws Throwable {{")?;

		// Call method
		let mut need_leading_comma = false;
		let mut method_call = format!("{}_handle.invokeExact(", &method.name);
		for arg in method.sig.1.iter() {
			if need_leading_comma {
				write!(&mut method_call, ", ")?;
			}
			need_leading_comma = true;

			match arg.kind {
				ArgKind::Primitive => write!(&mut method_call, "{}", arg.ident)?,
				ArgKind::Address => write!(&mut method_call, "{}.get().value", arg.ident)?,
				ArgKind::Boxed => write!(&mut method_call, "{}.value", arg.ident)?,
			};
		}
		write!(&mut method_call, ")")?;

		if let Some(ref ret_type) = method.sig.0 {
			match ret_type.kind {
				ArgKind::Primitive => write!(
					&mut file,
					"		return ({}){};",
					canoncalize_type(ret_type).unwrap(),
					method_call
				)?,
				ArgKind::Boxed => write!(
					&mut file,
					"		return new {}((MemorySegment){});",
					canoncalize_type(ret_type).unwrap(),
					method_call
				)?,
				ArgKind::Address => unimplemented!(),
			}
		} else {
			write!(&mut file, "		{};", method_call)?;
		}

		writeln!(
			&mut file,
			"
	}}"
		)?;
	}

	writeln!(&mut file, "}}")?;

	Ok(())
}

fn write_files(output_dir: &Path) -> Result<(), Box<dyn Error>> {
	fn write(
		output_dir: &Path,
		name: &'static str,
		contents: &'static str,
	) -> Result<(), Box<dyn Error>> {
		let mut path = output_dir.to_path_buf();
		path.push(name);

		let mut file = fs::OpenOptions::new()
			.create(true)
			.truncate(true)
			.write(true)
			.open(path)?;
		write!(&mut file, "{}", contents)?;
		Ok(())
	}

	write(output_dir, "RefCell.java", include_str!("RefCell.java"))?;
	write(output_dir, "Ref.java", include_str!("Ref.java"))?;
	write(output_dir, "RefMut.java", include_str!("RefMut.java"))?;
	write(
		output_dir,
		"BorrowException.java",
		include_str!("BorrowException.java"),
	)?;

	Ok(())
}
