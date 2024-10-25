use core::{iter, ops::Deref};

use proc_macro::TokenStream as CompilerTokenStream;
use proc_macro2::Span;
use punctuated::Punctuated;
use quote::{quote, ToTokens};
use spanned::Spanned;
use syn::*;

const PRIMITIVES: &[&str] = &[
	"i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "char",
];

fn primitive(path: &TypePath) -> Option<&'static str> {
	if path.path.segments.len() == 1 {
		let type_name = format!("{}", path.path.segments.first().unwrap().ident);

		PRIMITIVES
			.iter()
			.find(|x| **x == type_name.deref())
			.copied()
	} else {
		None
	}
}

fn error(msg: &'static str) -> CompilerTokenStream {
	let lit = Lit::Str(LitStr::new(msg, Span::call_site()));
	quote!(compile_error!(#lit)).into()
}

// Turn T into Box<T>
fn boxify_type(t: Type) -> Box<Type> {
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

fn unboxify_value(e: Expr) -> Expr {
	Expr::Unary(ExprUnary {
		attrs: Vec::new(),
		op: UnOp::Deref(Default::default()),
		expr: Box::new(e),
	})
}

fn ident_to_expr(pattype: &PatType) -> Option<Expr> {
	if let Pat::Ident(ident) = &*pattype.pat {
		Some(Expr::Path(ExprPath {
			attrs: Vec::new(),
			qself: None,
			path: Path {
				leading_colon: None,
				segments: iter::once(PathSegment {
					ident: ident.ident.clone(),
					arguments: PathArguments::None,
				})
				.collect(),
			},
		}))
	} else {
		None
	}
}

#[proc_macro_attribute]
pub fn java_export(_attr: CompilerTokenStream, input: CompilerTokenStream) -> CompilerTokenStream {
	let mut input: ItemFn = parse_macro_input!(input);

	let real_fn = input.clone();

	// Add "_" to stub function
	input.sig.ident = Ident::new(&format!("_{}", input.sig.ident), input.sig.span());
	// Change symbol name to original name
	input.attrs.push(Attribute {
		pound_token: Default::default(),
		style: AttrStyle::Outer,
		bracket_token: Default::default(),
		meta: Meta::NameValue(MetaNameValue {
			path: Path {
				leading_colon: None,
				segments: iter::once(PathSegment {
					ident: Ident::new("export_name", Span::call_site()),
					arguments: PathArguments::None,
				})
				.collect(),
			},
			eq_token: Default::default(),
			value: Expr::Lit(ExprLit {
				attrs: Vec::new(),
				lit: Lit::Str(LitStr::new(
					&real_fn.sig.ident.to_string(),
					Span::call_site(),
				)),
			}),
		}),
	});
	// Add extern
	input.sig.abi = Some(Abi {
		extern_token: Default::default(),
		name: Some(LitStr::new("C", Span::call_site())),
	});

	// Expressions used to call the real function from the stub
	let mut stub_translate_args: Punctuated<Expr, token::Comma> = Punctuated::new();

	// Rewrite args
	for arg in input.sig.inputs.iter_mut() {
		match arg {
			// foo: Type
			FnArg::Typed(t) => match &*t.ty {
				// e.g. ::crate::foo, i32
				Type::Path(path) => {
					if primitive(path).is_none() {
						stub_translate_args.push(unboxify_value(match ident_to_expr(t) {
							Some(x) => x,
							None => return error("bad ident in path type"),
						}));

						// Change from Type to Ptr<Type>
						t.ty = boxify_type(*t.ty.clone());
					}
				}
				Type::Reference(_) | Type::Ptr(_) => {
					stub_translate_args.push(match ident_to_expr(t) {
						Some(x) => x,
						None => return error("bad ident"),
					})
				}
				_ => return error("Only pathes, references, and pointers are supported"),
			},
			// self
			FnArg::Receiver(_) => return error("self is unimplemented"),
		}
	}

	// Possibly box return type
	if let ReturnType::Type(_, ref mut t) = input.sig.output {
		if let Type::Path(path) = &**t {
			if primitive(path).is_none() {
				*t = boxify_type(*t.clone());
			}
		}
	}

	// Stub function body
	{
		let stub_call = ExprCall {
			attrs: Vec::new(),
			func: Box::new(Expr::Path(ExprPath {
				attrs: Vec::new(),
				qself: None,
				path: Path {
					leading_colon: None,
					segments: iter::once(PathSegment {
						ident: real_fn.sig.ident.clone(),
						arguments: PathArguments::None,
					})
					.collect(),
				},
			})),
			paren_token: Default::default(),
			args: stub_translate_args,
		};

		//let expr = Expr::Call(stub_call);
		let expr = Expr::MethodCall(ExprMethodCall {
			attrs: Vec::new(),
			receiver: Box::new(Expr::Call(stub_call)),
			dot_token: Default::default(),
			method: Ident::new("into", Span::call_site()),
			turbofish: None,
			paren_token: Default::default(),
			args: Punctuated::new(),
		});

		input.block.stmts = vec![Stmt::Expr(expr, None)];
	}

	let mut stream = input.to_token_stream();
	stream.extend(real_fn.to_token_stream());
	stream.into()
}
