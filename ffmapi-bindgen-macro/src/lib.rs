use core::iter;

use ffmapi_bindgen_common::*;
use proc_macro::TokenStream as CompilerTokenStream;
use proc_macro2::*;
use punctuated::Punctuated;
use quote::{quote, ToTokens};
use spanned::Spanned;
use syn::*;

#[proc_macro_attribute]
pub fn java_export(_attr: CompilerTokenStream, input: CompilerTokenStream) -> CompilerTokenStream {
	let input: ItemFn = parse_macro_input!(input);

	match java_export_inner(input) {
		Ok(toks) => toks.into(),
		Err(e) => e.into_compile_error().into(),
	}
}

fn java_export_inner(input: ItemFn) -> Result<TokenStream> {
	let mut new_fn = input.clone();

	// Add "_" to stub function
	new_fn.sig.ident = Ident::new(&format!("_{}", new_fn.sig.ident), new_fn.sig.span());
	// Change symbol name to original name
	new_fn.attrs.push(Attribute {
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
				lit: Lit::Str(LitStr::new(&input.sig.ident.to_string(), Span::call_site())),
			}),
		}),
	});
	// Add extern
	new_fn.sig.abi = Some(Abi {
		extern_token: Default::default(),
		name: Some(LitStr::new("C", Span::call_site())),
	});

	// Maybe rewrite new_fn to use out parameter rather than return type
	if let ReturnType::Type(_, ref mut return_type) = new_fn.sig.output {
		let kind = match ArgKind::from_type(return_type) {
			Some(x) => x,
			None => return Err(Error::new(new_fn.sig.output.span(), "Invalid return type")),
		};

		if kind == ArgKind::Boxed {
			*return_type = boxify_type(*return_type.clone());
		}
	}

	let parsed_args = extract_args(&new_fn)?;

	// Maybe rewrite new_fn args to box types
	for (args, arg_ast) in parsed_args.iter().zip(new_fn.sig.inputs.iter_mut()) {
		if args.kind == ArgKind::Boxed {
			let pat_type = match arg_ast {
				FnArg::Typed(t) => t,
				_ => unreachable!(),
			};

			pat_type.ty = boxify_type(*pat_type.ty.clone());
		}
	}

	// Expressions used to call the real function from the stub
	let stub_translate_args: Punctuated<Expr, token::Comma> = parsed_args
		.iter()
		.map(|arg| {
			let path_expr = Expr::Path(ExprPath {
				attrs: Vec::new(),
				qself: None,
				path: Path {
					leading_colon: None,
					segments: iter::once(PathSegment {
						ident: arg.ident.clone(),
						arguments: PathArguments::None,
					})
					.collect(),
				},
			});

			match arg.kind {
				ArgKind::Boxed => Expr::Unary(ExprUnary {
					attrs: Vec::new(),
					op: UnOp::Deref(Default::default()),
					expr: Box::new(path_expr),
				}),
				ArgKind::Address | ArgKind::Primitive => path_expr,
			}
		})
		.collect();

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
						ident: input.sig.ident.clone(),
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

		new_fn.block.stmts = vec![Stmt::Expr(expr, None)];
	}

	let mut stream = new_fn.to_token_stream();
	stream.extend(input.to_token_stream());

	// Safety checks on arguments.
	stream.extend(parsed_args.iter().flat_map(|arg| {
		if arg.kind == ArgKind::Boxed {
			let ty = &arg.ty;
			Some(quote! {
				const _: () = {
					trait SendPlusSync: Send + Sync {}
					impl SendPlusSync for #ty {}
				};
			})
		} else {
			None
		}
	}));

	Ok(stream)
}
