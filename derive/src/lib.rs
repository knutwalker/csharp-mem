/// A derive macro that can be used to bind to a .NET class. This allows reading
/// the contents of an instance of the class described by the struct from a
/// process. Each field must match the name of the field in the class exactly
/// (or alternatively renamed with the `#[rename = "..."]` attribute) and needs
/// to be of a type that can be read from a process. Fields can be marked as
/// static with the `#[static_field]` attribute.
///
/// ## Difference to `derive(asr::game_engine::unity::il2cpp::Class)`
///
/// * The `rename` attribute is supported on the struct/class level.
/// * Classes cannot have mixed static and non-static fields.
/// * A new `singleton` attribute to mark a static singleton field for an
///   otherwise non-static class.
/// * The binding is resolved lazily, which results in different methods
///     * `bind` has no parameters, is not `async` and always succeeds
///     * `read` has the parameters that `bind` would have with `derive(Class)`
///        that is, `&Process`, `&Module`, `&Image` and optionally an instance
///     * If the class has only static fields, or one of the fields is marked
///       as `singleton`, `read` does not take an instance argument.
///
/// ### The `rename` attribute is supported on the struct/class level
///
/// ```no_run
/// #[derive(Class2)]
/// #[rename = "Timer"]
/// struct MyTimer {
///     time: f32,
/// }
/// ```
///
/// This will bind to a .NET class called `Timer`
///
/// ### Classes cannot have mixed static and non-static fields.
///
/// Use two structs instead:
///
/// ```no_run
/// #[derive(Class2)]
/// struct Timer {
///     #[rename = "currentLevelTime"]
///     level_time: f32,
/// }
///
/// #[derive(Class2)]
/// #[rename = "Timer"]
/// struct TimerStatic {
///     #[static_field]
///     foo: bool,
/// }
///
/// ### The binding is resolved lazily
///
/// The class can then be bound to the process like so:
///
/// ```no_run
/// let timer_class = Timer::bind().await;
/// ```
///
/// Once you have an instance, you can read the instance from the process like
/// so:
///
/// ```no_run
/// if let Ok(timer) = timer_class.read(&process, &module, &image, timer_instance) {
///     // Do something with the instance.
/// }
/// ```
#[cfg(feature = "il2cpp")]
#[proc_macro_derive(Il2cppClass, attributes(static_field, singleton, rename))]
pub fn il2cpp_class_binding(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    process(input, quote::quote! { ::asr::game_engine::unity::il2cpp })
}

/// A derive macro that can be used to bind to a .NET class. This allows reading
/// the contents of an instance of the class described by the struct from a
/// process. Each field must match the name of the field in the class exactly
/// (or alternatively renamed with the `#[rename = "..."]` attribute) and needs
/// to be of a type that can be read from a process. Fields can be marked as
/// static with the `#[static_field]` attribute.
///
/// ## Difference to `derive(asr::game_engine::unity::mono::Class)`
///
/// * The `rename` attribute is supported on the struct/class level.
/// * Classes cannot have mixed static and non-static fields.
/// * A new `singleton` attribute to mark a static singleton field for an
///   otherwise non-static class.
/// * The binding is resolved lazily, which results in different methods
///     * `bind` has no parameters, is not `async` and always succeeds
///     * `read` has the parameters that `bind` would have with `derive(Class)`
///        that is, `&Process`, `&Module`, `&Image` and optionally an instance
///     * If the class has only static fields, or one of the fields is marked
///       as `singleton`, `read` does not take an instance argument.
///
/// ### The `rename` attribute is supported on the struct/class level
///
/// ```no_run
/// #[derive(Class2)]
/// #[rename = "Timer"]
/// struct MyTimer {
///     time: f32,
/// }
/// ```
///
/// This will bind to a .NET class called `Timer`
///
/// ### Classes cannot have mixed static and non-static fields.
///
/// Use two structs instead:
///
/// ```no_run
/// #[derive(Class2)]
/// struct Timer {
///     #[rename = "currentLevelTime"]
///     level_time: f32,
/// }
///
/// #[derive(Class2)]
/// #[rename = "Timer"]
/// struct TimerStatic {
///     #[static_field]
///     foo: bool,
/// }
///
/// ### The binding is resolved lazily
///
/// The class can then be bound to the process like so:
///
/// ```no_run
/// let timer_class = Timer::bind().await;
/// ```
///
/// Once you have an instance, you can read the instance from the process like
/// so:
///
/// ```no_run
/// if let Ok(timer) = timer_class.read(&process, &module, &image, timer_instance) {
///     // Do something with the instance.
/// }
/// ```
#[cfg(feature = "mono")]
#[proc_macro_derive(MonoClass, attributes(static_field, singleton, rename))]
pub fn mono_class_binding(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    process(input, quote::quote! { ::asr::game_engine::unity::mono })
}

#[cfg(any(feature = "mono", feature = "il2cpp"))]
fn process(
    input: proc_macro::TokenStream,
    mono_module: impl quote::ToTokens,
) -> proc_macro::TokenStream {
    match inner::process(input, mono_module) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

#[cfg(any(feature = "mono", feature = "il2cpp"))]
mod inner {
    use proc_macro2::TokenStream;
    use quote::{quote, ToTokens};
    use syn::{Attribute, Data, DeriveInput, Expr, ExprLit, Ident, Lit};

    struct FieldSpec {
        is_singleton: bool,
        field_name: Ident,
        binding_name: Ident,
        lookup_name: String,
    }

    pub fn process(
        input: proc_macro::TokenStream,
        mono_module: impl ToTokens,
    ) -> syn::Result<TokenStream> {
        let ast: DeriveInput = syn::parse(input).unwrap();

        let struct_data = match ast.data {
            Data::Struct(s) => s,
            _ => {
                return Err(syn::Error::new(
                    ast.ident.span(),
                    "Only structs are supported.",
                ))
            }
        };

        let class_name = ast
            .attrs
            .into_iter()
            .find_map(|o| parse_rename(&o))
            .unwrap_or_else(|| Ok(ast.ident.to_string()))?;

        let struct_name = ast.ident;
        let binding_name = Ident::new(&format!("{struct_name}Binding"), struct_name.span());

        let mut static_specs = Vec::new();
        let mut non_static_specs = Vec::<FieldSpec>::new();

        for field in struct_data.fields {
            let field_ident = field.ident.as_ref().ok_or_else(|| {
                syn::Error::new(struct_name.span(), "Cannot have unnamed fields.")
            })?;

            let is_static = field
                .attrs
                .iter()
                .any(|o| o.path().is_ident("static_field"));

            let is_singleton = field.attrs.iter().any(|o| o.path().is_ident("singleton"));

            if is_singleton && is_static {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "Singleton fields are implied to be static, both attributes are invalid.",
                ));
            }

            if is_singleton && non_static_specs.iter().any(|o| o.is_singleton) {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "Cannot have more than one singleton field in a struct",
                ));
            }

            let field_name = field.ident.clone().unwrap();
            let binding_name =
                Ident::new(&format!("__internal_field_{field_name}"), field_name.span());

            let lookup_name = field
                .attrs
                .iter()
                .find_map(parse_rename)
                .unwrap_or_else(|| Ok(field_name.to_string()))?;

            let spec = FieldSpec {
                is_singleton,
                field_name,
                binding_name,
                lookup_name,
            };

            if is_static {
                static_specs.push(spec);
            } else {
                non_static_specs.push(spec);
            }
        }

        Ok(
            match (static_specs.is_empty(), non_static_specs.is_empty()) {
                (false, false) => {
                    return Err(syn::Error::new(
                        struct_name.span(),
                        "Cannot have both static and non-static fields in a struct",
                    ));
                }
                (_, true) => static_binding(
                    struct_name,
                    binding_name,
                    class_name,
                    static_specs,
                    mono_module.into_token_stream(),
                ),
                (true, false) => non_static_binding(
                    struct_name,
                    binding_name,
                    class_name,
                    non_static_specs,
                    mono_module.into_token_stream(),
                ),
            },
        )
    }

    fn static_binding(
        struct_name: Ident,
        generate_struct: Ident,
        lookup_class: String,
        fields: Vec<FieldSpec>,
        mono_module: TokenStream,
    ) -> TokenStream {
        let fields = fields
        .into_iter()
        .map(
            |FieldSpec {
                 field_name,
                 binding_name,
                 lookup_name,
                 ..
             }| {
                FieldDef {
                    name: field_name,
                    typ: quote! { ::core::option::Option<asr::Address>},
                    lookup: quote! { class.get_static_field(game.process(), game.module(), #lookup_name)? },
                    read: quote! { #binding_name },
                    binding: binding_name,
                }
            },
        )
        .collect();

        generate_binding(
            mono_module,
            struct_name,
            generate_struct,
            lookup_class,
            quote! {},
            fields,
        )
    }

    fn non_static_binding(
        struct_name: Ident,
        generate_struct: Ident,
        lookup_class: String,
        mut fields: Vec<FieldSpec>,
        mono_module: TokenStream,
    ) -> TokenStream {
        const SINGLETON_NAME: &str = "__internal_instance__";

        fields.sort_by_key(|o| !o.is_singleton);

        let singleton_name = fields.get(0).and_then(|o| {
            o.is_singleton
                .then(|| Ident::new(SINGLETON_NAME, o.field_name.span()))
        });

        let fields = fields
        .into_iter()
        .map(
            |FieldSpec {
                 is_singleton,
                 field_name,
                 binding_name,
                 lookup_name,
            }| {
                if is_singleton {
                    let name = singleton_name.as_ref().unwrap();
                    FieldDef {
                        name: field_name,
                        typ: quote! { ::core::option::Option<asr::Address> },
                        lookup: quote! { class.get_static_field(game.process(), game.module(), #lookup_name)? },
                        read: quote! { #name },
                        binding: name.clone(),
                    }
                } else {

                    FieldDef {
                        name: field_name,
                        typ: quote! { ::core::option::Option<::core::num::NonZeroU32> },
                        lookup: quote! {
                            ::core::num::NonZeroU32::new(class.get_field(game.process(), game.module(), #lookup_name)?)
                                .expect("A field with offset 0 in a unity project is not valid")
                        },
                        read: match singleton_name.as_ref() {
                            Some(instance) =>quote! { ::asr::Address::from(#instance) + #binding_name.get() },
                            None => quote! { instance + #binding_name.get() },
                        },
                        binding: binding_name,
                    }
                }
            },
        )
        .collect();

        let instance_param = if singleton_name.is_some() {
            quote! {}
        } else {
            quote! { instance: ::asr::Address, }
        };

        generate_binding(
            mono_module,
            struct_name,
            generate_struct,
            lookup_class,
            instance_param,
            fields,
        )
    }

    struct FieldDef {
        name: Ident,
        typ: TokenStream,
        lookup: TokenStream,
        read: TokenStream,
        binding: Ident,
    }

    fn generate_binding(
        mono_module: TokenStream,
        struct_name: Ident,
        generate_struct: Ident,
        lookup_class: String,
        additional_params: TokenStream,
        fields2: Vec<FieldDef>,
    ) -> TokenStream {
        let mut field_names = Vec::new();
        let mut field_types = Vec::new();
        let mut binding_names = Vec::new();
        let mut lookups = Vec::new();
        let mut reads = Vec::new();

        for field in fields2 {
            field_names.push(field.name);
            field_types.push(field.typ);
            binding_names.push(field.binding);
            lookups.push(field.lookup);
            reads.push(field.read);
        }

        let read_pointer = if additional_params.is_empty() {
            quote! {}
        } else {
            quote! {
                pub fn read_pointer(
                    &mut self,
                    game: &::csharp_mem::Game<'_>,
                    pointer: ::csharp_mem::Pointer<#struct_name>,
                ) -> Option<#struct_name> {
                    self.read(game, pointer.address().into())
                }
            }
        };

        let read_impl = if field_names.is_empty() {
            quote! {}
        } else {
            quote! {
                pub fn read(
                    &mut self,
                    game: &::csharp_mem::Game<'_>,
                    #additional_params
                ) -> ::core::option::Option<#struct_name> {
                    let class = match self.class {
                        ::core::option::Option::Some(ref cls) => cls,
                        ::core::option::Option::None => {
                            let class = game.image().get_class(game.process(), game.module(), #lookup_class)?;
                            self.class = ::core::option::Option::Some(class);
                            self.class.as_ref().unwrap()
                        }
                    };

                    #(
                        let #binding_names = match self.#field_names {
                            ::core::option::Option::Some(field) => field,
                            ::core::option::Option::None => {
                                let field = #lookups;
                                self.#field_names = ::core::option::Option::Some(field);
                                field
                            }
                        };
                    )*

                    #(
                        let #binding_names = game.process().read(#reads).map_err(drop).ok()?;
                    )*

                    ::core::option::Option::Some(#struct_name {#(#field_names: #binding_names,)*})
                }
            }
        };

        quote! {
            struct #generate_struct {
                class: ::core::option::Option<#mono_module::Class>,
                #(#field_names: #field_types,)*
            }

            impl #generate_struct {
                pub fn class(
                    &mut self,
                    game: &::csharp_mem::Game<'_>,
                ) -> ::core::option::Option<&#mono_module::Class> {
                    let class = match self.class {
                        ::core::option::Option::Some(ref cls) => cls,
                        ::core::option::Option::None => {
                            let class = game.image().get_class(game.process(), game.module(), #lookup_class)?;
                            self.class = ::core::option::Option::Some(class);
                            self.class.as_ref().unwrap()
                        }
                    };

                    ::core::option::Option::Some(class)
                }

                #read_impl

                #read_pointer
            }

            impl #struct_name {
                fn bind() -> #generate_struct {
                    #generate_struct {
                        class: ::core::option::Option::None,
                        #(#field_names: ::core::option::Option::None,)*
                    }
                }
            }
        }
    }
    fn parse_rename(attr: &Attribute) -> Option<syn::Result<String>> {
        attr.path()
            .is_ident("rename")
            .then(|| {
                attr.meta
                    .require_name_value()
                    .map(|o| match &o.value {
                        Expr::Lit(ExprLit {
                            lit: Lit::Str(name),
                            ..
                        }) => Some(name.value()),
                        _ => None,
                    })
                    .transpose()
            })
            .flatten()
    }
}
