use core::{iter, ops::Deref};

use proc_macro2::*;
use syn::*;

pub const PRIMITIVES: &[&str] = &[
	"i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "char",
];

pub fn primitive_match(path: &TypePath) -> Option<&'static str> {
	if path.path.segments.len() == 1 {
		let type_name = format!("{}", path.path.segments.first()?.ident);

		PRIMITIVES
			.iter()
			.find(|x| **x == type_name.deref())
			.copied()
	} else {
		None
	}
}

// Turn T into Box<T>
pub fn boxify_type(t: Type) -> Box<Type> {
	Box::new(Type::Path(TypePath {
		qself: None,
		path: Path {
			leading_colon: None,
			segments: iter::once(PathSegment {
				ident: Ident::new("Box", Span::call_site()),
				arguments: PathArguments::AngleBracketed(AngleBracketedGenericArguments {
					colon2_token: None,
					lt_token: Default::default(),
					args: iter::once(GenericArgument::Type(t)).collect(),
					gt_token: Default::default(),
				}),
			})
			.collect(),
		},
	}))
}

fn get_pat_ident(pattern: &Pat) -> Option<&Ident> {
	match pattern {
		Pat::Ident(pat_ident) => Some(&pat_ident.ident),
		_ => None,
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
	Primitive,
	Address,
	Boxed,
}

impl ArgKind {
	pub fn from_type(ty: &Type) -> Option<Self> {
		match ty {
			Type::Path(ref path) => {
				if primitive_match(path).is_some() {
					Some(ArgKind::Primitive)
				} else {
					Some(ArgKind::Boxed)
				}
			}

			Type::Reference(_) | Type::Ptr(_) => Some(ArgKind::Address),

			_ => None,
		}
	}
}

#[derive(Debug, Clone)]
pub struct RustFnArg {
	pub ident: Ident,
	pub kind: ArgKind,
	pub ty: Type,
}

pub fn extract_args(input: &ItemFn) -> Result<Vec<RustFnArg>> {
	let mut args = Vec::with_capacity(input.sig.inputs.len());
	for raw_arg in input.sig.inputs.iter() {
		let arg = match raw_arg {
			FnArg::Receiver(x) => return Err(Error::new_spanned(x, "self is not supported yet")),
			FnArg::Typed(t) => {
				let ident = match get_pat_ident(&t.pat) {
					Some(x) => x.clone(),
					None => return Err(Error::new_spanned(t, "Unsupported pattern expression")),
				};

				let ty = (*t.ty).clone();
				let kind = ArgKind::from_type(&ty);

				match kind {
					Some(kind) => RustFnArg { ident, kind, ty },
					None => {
						return Err(Error::new_spanned(
							(*t.ty).clone(),
							"Only path types, references, and pointers are supported",
						))
					}
				}
			}
		};

		args.push(arg);
	}

	Ok(args)
}

use syn::{ItemFn, ReturnType, Type, TypePath, Error};

pub fn extract_return_type(input: &ItemFn) -> Result<Option<RustFnArg>> {
	match &input.sig.output {
		ReturnType::Default => Ok(None), // No return type, so return None
		ReturnType::Type(_, boxed_ty) => {
			// Unbox the return type
			let ty = &**boxed_ty;
			let kind = ArgKind::from_type(ty);

			// If the return type is supported, create a RustFnArg with a placeholder ident
			match kind {
				Some(kind) => Ok(Some(RustFnArg {
					ident: Ident::new("return_value", proc_macro2::Span::call_site()), // Placeholder ident
					kind,
					ty: ty.clone(),
				})),
				None => Err(Error::new_spanned(
					ty,
					"Unsupported return type: only primitives, references, and boxed types are allowed.",
				)),
			}
		}
	}
}
