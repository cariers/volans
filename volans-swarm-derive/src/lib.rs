use heck::ToUpperCamelCase;
use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Data, DataStruct, DeriveInput, Expr, ExprLit, Lit, Meta, Token, parse_macro_input,
    punctuated::Punctuated,
};

trait RequireStrLit {
    fn require_str_lit(&self) -> syn::Result<String>;
}

impl RequireStrLit for Expr {
    fn require_str_lit(&self) -> syn::Result<String> {
        match self {
            Expr::Lit(ExprLit {
                lit: Lit::Str(str), ..
            }) => Ok(str.value()),
            _ => Err(syn::Error::new_spanned(self, "expected a string literal")),
        }
    }
}

#[proc_macro_derive(NetworkIncomingBehavior, attributes(behavior))]
pub fn network_incoming_macro_derive(input: TokenStream) -> TokenStream {
    // 解析输入的 AST
    let ast = parse_macro_input!(input as DeriveInput);
    build_incoming(&ast).unwrap_or_else(|e| e.to_compile_error().into())
}

#[proc_macro_derive(NetworkOutgoingBehavior, attributes(behavior))]
pub fn network_outgoing_macro_derive(input: TokenStream) -> TokenStream {
    // 解析输入的 AST
    let ast = parse_macro_input!(input as DeriveInput);
    build_outgoing(&ast).unwrap_or_else(|e| e.to_compile_error().into())
}

fn build_incoming(ast: &DeriveInput) -> syn::Result<TokenStream> {
    match ast.data {
        // 只能解析结构体
        Data::Struct(ref s) => build_incoming_struct(ast, s),
        Data::Enum(_) => Err(syn::Error::new_spanned(
            ast,
            "Cannot derive `NetworkIncomingBehavior` on enums",
        )),
        Data::Union(_) => Err(syn::Error::new_spanned(
            ast,
            "Cannot derive `NetworkIncomingBehavior` on union",
        )),
    }
}

fn build_outgoing(ast: &DeriveInput) -> syn::Result<TokenStream> {
    match ast.data {
        // 只能解析结构体
        Data::Struct(ref s) => build_outgoing_struct(ast, s),
        Data::Enum(_) => Err(syn::Error::new_spanned(
            ast,
            "Cannot derive `NetworkOutgoingBehavior` on enums",
        )),
        Data::Union(_) => Err(syn::Error::new_spanned(
            ast,
            "Cannot derive `NetworkOutgoingBehavior` on union",
        )),
    }
}

struct PreludeTokenStream {
    url: proc_macro2::TokenStream,
    peer_id: proc_macro2::TokenStream,
    behavior_event: proc_macro2::TokenStream,
    listener_event: proc_macro2::TokenStream,
    connection_id: proc_macro2::TokenStream,
    connection_denied: proc_macro2::TokenStream,
    network_behavior_to_impl: proc_macro2::TokenStream,
    network_incoming_behavior_to_impl: proc_macro2::TokenStream,
    network_outgoing_behavior_to_impl: proc_macro2::TokenStream,
    handler_select: proc_macro2::TokenStream,
    t_handler: proc_macro2::TokenStream,
    t_handler_event: proc_macro2::TokenStream,
    t_handler_action: proc_macro2::TokenStream,
    connection_handler: proc_macro2::TokenStream,
    // inbound_stream_handler: proc_macro2::TokenStream,
    // outbound_stream_handler: proc_macro2::TokenStream,
    either: proc_macro2::TokenStream,

    dial_opts: proc_macro2::TokenStream,

    // error
    connection_error: proc_macro2::TokenStream,
    listen_error: proc_macro2::TokenStream,
    dial_error: proc_macro2::TokenStream,

    impl_generics: proc_macro2::TokenStream,
}

struct CommonParsed {
    prelude: PreludeTokenStream,
    attributes: BehaviorAttributes,
}

fn parse_common_token_stream(ast: &DeriveInput) -> syn::Result<CommonParsed> {
    let attributes = parse_attributes(ast)?;
    let BehaviorAttributes { prelude_path, .. } = &attributes;

    let impl_generics = {
        let tp = ast.generics.type_params();
        let lf = ast.generics.lifetimes();
        let cst = ast.generics.const_params();
        quote! {<#(#lf,)* #(#tp,)* #(#cst,)*>}
    };

    let prelude = PreludeTokenStream {
        url: quote! { #prelude_path::Url },
        peer_id: quote! { #prelude_path::PeerId },
        behavior_event: quote! { #prelude_path::BehaviorEvent },
        listener_event: quote! { #prelude_path::ListenerEvent },
        connection_id: quote! { #prelude_path::ConnectionId },
        connection_denied: quote! { #prelude_path::ConnectionDenied },
        network_behavior_to_impl: quote! { #prelude_path::NetworkBehavior },
        network_incoming_behavior_to_impl: quote! { #prelude_path::NetworkIncomingBehavior },
        network_outgoing_behavior_to_impl: quote! { #prelude_path::NetworkOutgoingBehavior },
        handler_select: quote! { #prelude_path::ConnectionHandlerSelect },
        t_handler: quote! { #prelude_path::THandler },
        t_handler_event: quote! { #prelude_path::THandlerEvent },
        t_handler_action: quote! { #prelude_path::THandlerAction },
        connection_handler: quote! { #prelude_path::ConnectionHandler },
        // inbound_stream_handler: quote! { #prelude_path::InboundStreamHandler },
        // outbound_stream_handler: quote! { #prelude_path::OutboundStreamHandler },
        either: quote! { #prelude_path::Either },
        connection_error: quote! { #prelude_path::ConnectionError },
        listen_error: quote! { #prelude_path::ListenError },
        dial_error: quote! { #prelude_path::DialError },
        dial_opts: quote! { #prelude_path::DialOpts },
        impl_generics,
    };

    Ok(CommonParsed {
        prelude,
        attributes,
    })
}

fn build_event_impl(
    ast: &DeriveInput,
    data_struct: &DataStruct,
    common: &CommonParsed,
) -> (
    syn::Type,
    Option<proc_macro2::TokenStream>,
    Vec<proc_macro2::TokenStream>,
) {
    let CommonParsed {
        prelude:
            PreludeTokenStream {
                network_behavior_to_impl,
                impl_generics,
                ..
            },
        attributes:
            BehaviorAttributes {
                user_specified_out_event,
                ..
            },
    } = common;

    // 结构体名称
    let name = &ast.ident;
    // ty_generics: 泛型参数, where_clause: where 子句
    let (_, ty_generics, where_clause) = ast.generics.split_for_impl();

    match user_specified_out_event {
        Some(name) => {
            let definition = None;
            let from_clauses = data_struct
                .fields
                .iter()
                .map(|field| {
                    let ty = &field.ty;
                    quote! {#name: From< <#ty as #network_behavior_to_impl>::Event >}
                })
                .collect::<Vec<_>>();
            (name.clone(), definition, from_clauses)
        }
        None => {
            let enum_name_str = ast.ident.to_string() + "Event";
            let enum_name: syn::Type =
                syn::parse_str(&enum_name_str).expect("ident + `Event` is a valid type");
            let definition = {
                let fields = data_struct.fields.iter().map(|field| {
                    let variant: syn::Variant = syn::parse_str(
                        &field
                            .ident
                            .clone()
                            .expect("Fields of NetworkBehaviour implementation to be named.")
                            .to_string()
                            .to_upper_camel_case(),
                    )
                    .expect("uppercased field name to be a valid enum variant");
                    let ty = &field.ty;
                    (variant, ty)
                });

                let enum_variants = fields.clone().map(
                    |(variant, ty)| quote! {#variant(<#ty as #network_behavior_to_impl>::Event)},
                );

                let visibility = &ast.vis;

                let additional = fields
                    .clone()
                    .map(|(_variant, tp)| quote! { #tp : #network_behavior_to_impl })
                    .collect::<Vec<_>>();

                let additional_debug = fields
                        .clone()
                        .map(|(_variant, ty)| quote! { <#ty as #network_behavior_to_impl>::Event : ::core::fmt::Debug })
                        .collect::<Vec<_>>();

                let where_clause = {
                    if let Some(where_clause) = where_clause {
                        if where_clause.predicates.trailing_punct() {
                            Some(quote! {#where_clause #(#additional),* })
                        } else {
                            Some(quote! {#where_clause, #(#additional),*})
                        }
                    } else if additional.is_empty() {
                        None
                    } else {
                        Some(quote! {where #(#additional),*})
                    }
                };

                let where_clause_debug = where_clause
                    .as_ref()
                    .map(|where_clause| quote! {#where_clause, #(#additional_debug),*});

                let match_variants = fields.map(|(variant, _ty)| variant);
                let msg = format!("`NetworkBehavior::Event` produced by {name}.");

                Some(quote! {
                    #[doc = #msg]
                    #visibility enum #enum_name #impl_generics
                        #where_clause
                    {
                        #(#enum_variants),*
                    }

                    impl #impl_generics ::core::fmt::Debug for #enum_name #ty_generics #where_clause_debug {
                        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
                            match &self {
                                #(#enum_name::#match_variants(event) => {
                                    write!(f, "{}: {:?}", #enum_name_str, event)
                                }),*
                            }
                        }
                    }
                })
            };
            let from_clauses = vec![];
            (enum_name, definition, from_clauses)
        }
    }
}

fn where_clause_token(
    ast: &DeriveInput,
    data_struct: &DataStruct,
    out_event_from_clauses: Vec<proc_macro2::TokenStream>,
    trait_to_impl: &proc_macro2::TokenStream,
) -> Option<proc_macro2::TokenStream> {
    let (_, _, where_clause) = ast.generics.split_for_impl();

    let where_clause = {
        let additional = data_struct
            .fields
            .iter()
            .map(|field| {
                let ty = &field.ty;
                quote! {#ty: #trait_to_impl}
            })
            .chain(out_event_from_clauses)
            .collect::<Vec<_>>();

        if let Some(where_clause) = where_clause {
            if where_clause.predicates.trailing_punct() {
                Some(quote! {#where_clause #(#additional),* })
            } else {
                Some(quote! {#where_clause, #(#additional),*})
            }
        } else {
            Some(quote! {where #(#additional),*})
        }
    };
    where_clause
}

fn build_network_behavior_impl(
    ast: &DeriveInput,
    data_struct: &DataStruct,
    common_parsed: &CommonParsed,
) -> (proc_macro2::TokenStream, Vec<proc_macro2::TokenStream>) {
    // 结构体名称
    let name = &ast.ident;
    // ty_generics: 泛型参数, where_clause: where 子句
    let (_, ty_generics, _) = ast.generics.split_for_impl();

    let (out_event_name, out_event_definition, out_event_from_clauses) =
        build_event_impl(ast, data_struct, common_parsed);

    let where_clause = where_clause_token(
        ast,
        data_struct,
        out_event_from_clauses.clone(),
        &common_parsed.prelude.network_behavior_to_impl,
    );

    let out_event_reference = if out_event_definition.is_some() {
        quote! { #out_event_name #ty_generics }
    } else {
        quote! { #out_event_name }
    };

    let CommonParsed {
        prelude:
            PreludeTokenStream {
                peer_id,
                behavior_event,
                connection_id,
                network_behavior_to_impl,
                handler_select,
                t_handler,
                t_handler_event,
                t_handler_action,
                either,
                impl_generics,
                ..
            },
        ..
    } = &common_parsed;

    let connection_handler_ty = {
        let mut ph_ty = None;
        for field in data_struct.fields.iter() {
            let ty = &field.ty;
            let field_info = quote! { #t_handler<#ty> };
            match ph_ty {
                Some(ev) => ph_ty = Some(quote! { #handler_select<#ev, #field_info> }),
                ref mut ev @ None => *ev = Some(field_info),
            }
        }
        ph_ty.unwrap_or(quote! {()})
    };

    let on_connection_handler_event_stmts = data_struct.fields.iter().enumerate().enumerate().map(
        |(enum_n, (field_n, field))| {
            let mut elem = if enum_n != 0 {
                quote! { #either::Right(ev) }
            } else {
                quote! { ev }
            };

            for _ in 0..data_struct.fields.len() - 1 - enum_n {
                elem = quote! { #either::Left(#elem) };
            }

            Some(match field.ident {
                Some(ref i) => quote! { #elem => {
                #network_behavior_to_impl::on_connection_handler_event(&mut self.#i, id, peer_id, ev) }},
                None => quote! { #elem => {
                #network_behavior_to_impl::on_connection_handler_event(&mut self.#field_n, id, peer_id, ev) }},
            })
        },
    );

    let poll_stmts = data_struct
        .fields
        .iter()
        .enumerate()
        .map(|(field_n, field)| {
            let field = field
                .ident
                .clone()
                .expect("Fields of NetworkBehavior implementation to be named.");

            let mut wrapped_event = if field_n != 0 {
                quote! { #either::Right(event) }
            } else {
                quote! { event }
            };
            for _ in 0..data_struct.fields.len() - 1 - field_n {
                wrapped_event = quote! { #either::Left(#wrapped_event) };
            }

            let map_event = if out_event_definition.is_some() {
                let event_variant: syn::Variant =
                    syn::parse_str(&field.to_string().to_upper_camel_case())
                        .expect("field name to be a valid enum variant name");
                quote! { #out_event_name::#event_variant }
            } else {
                quote! { |e| e.into() }
            };

            let map_handler_action = quote! { |event| #wrapped_event };

            quote! {
                match #network_behavior_to_impl::poll(&mut self.#field, cx) {
                    std::task::Poll::Ready(e) => return std::task::Poll::Ready(e.map_event(#map_event).map_handler_action(#map_handler_action)),
                    std::task::Poll::Pending => {},
                }
            }
        });

    let final_quote = quote! {
        #out_event_definition
        impl #impl_generics #network_behavior_to_impl for #name #ty_generics
        #where_clause
        {
            type ConnectionHandler = #connection_handler_ty;
            type Event = #out_event_reference;

            fn on_connection_handler_event(
                &mut self,
                id: #connection_id,
                peer_id: #peer_id,
                event: #t_handler_event<Self>
            ) {
                match event {
                    #(#on_connection_handler_event_stmts),*
                }
            }

            fn poll(
                &mut self,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<#behavior_event<Self::Event, #t_handler_action<Self>>> {
                #(#poll_stmts)*
                std::task::Poll::Pending
            }

        }
    };

    return (final_quote, out_event_from_clauses);
}

fn build_incoming_struct(ast: &DeriveInput, data_struct: &DataStruct) -> syn::Result<TokenStream> {
    let common_parsed = parse_common_token_stream(ast)?;
    // 结构体名称
    let name = &ast.ident;
    // ty_generics: 泛型参数, where_clause: where 子句
    let (_, ty_generics, _) = ast.generics.split_for_impl();

    let (network_behavior_token, out_event_from_clauses) =
        build_network_behavior_impl(ast, data_struct, &common_parsed);

    let CommonParsed {
        prelude:
            PreludeTokenStream {
                url,
                peer_id,
                listener_event,
                connection_id,
                connection_denied,
                network_incoming_behavior_to_impl,
                connection_handler,
                listen_error,
                connection_error,
                impl_generics,
                ..
            },
        ..
    } = &common_parsed;

    let where_clause = where_clause_token(
        ast,
        data_struct,
        out_event_from_clauses,
        network_incoming_behavior_to_impl,
    );

    // 生成 fn handle_pending_inbound_connection
    let handle_pending_inbound_connection_stmts =
        data_struct
            .fields
            .iter()
            .enumerate()
            .map(|(field_n, field)| {
                match field.ident {
                    Some(ref i) => quote! {
                        #network_incoming_behavior_to_impl::handle_pending_connection(&mut self.#i, id, local_addr, remote_addr)?;
                    },
                    None => quote! {
                        #network_incoming_behavior_to_impl::handle_pending_connection(&mut self.#field_n, id, local_addr, remote_addr)?;
                    }
                }
            });

    let handle_established_inbound_connection = {
        let mut out_handler = None;

        for (field_n, field) in data_struct.fields.iter().enumerate() {
            let field_name = match field.ident {
                Some(ref i) => quote! { self.#i },
                None => quote! { self.#field_n },
            };

            let builder = quote! {
                #field_name.handle_established_connection(id, peer_id, local_addr, remote_addr)?
            };

            match out_handler {
                Some(h) => out_handler = Some(quote! { #connection_handler::select(#h, #builder) }),
                ref mut h @ None => *h = Some(builder),
            }
        }

        out_handler.unwrap_or(quote! {()})
    };

    // 生成 on_listen_failure
    let on_listen_failure_stmts = data_struct.fields.iter().enumerate().map(
        |(field_n, field)| {
            match field.ident {
                Some(ref i) => quote! {
                    #network_incoming_behavior_to_impl::on_listen_failure(&mut self.#i, id, peer_id, local_addr, remote_addr, error);
                },
                None => quote! {
                    #network_incoming_behavior_to_impl::on_listen_failure(&mut self.#field_n, id, peer_id, local_addr, remote_addr, error);
                },
            }
        },
    );

    // 生成 on_connection_established
    let on_connection_established_stmts = data_struct.fields.iter().enumerate().map(
        |(field_n, field)| {
            match field.ident {
                Some(ref i) => quote! {
                    #network_incoming_behavior_to_impl::on_connection_established(&mut self.#i, id, peer_id, local_addr, remote_addr);
                },
                None => quote! {
                    #network_incoming_behavior_to_impl::on_connection_established(&mut self.#field_n, id, peer_id, local_addr, remote_addr);
                },
            }
        },
    );

    // 生成 on_connection_closed
    let on_connection_closed_stmts = data_struct.fields.iter().enumerate().map(
        |(field_n, field)| {
            match field.ident {
                Some(ref i) => quote! {
                    #network_incoming_behavior_to_impl::on_connection_closed(&mut self.#i, id, peer_id, local_addr, remote_addr, reason);
                },
                None => quote! {
                    #network_incoming_behavior_to_impl::on_connection_closed(&mut self.#field_n, id, peer_id, local_addr, remote_addr, reason);
                },
            }
        },
    );

    // 生成 on_listener_event
    let on_listener_event_stmts = {
        data_struct
            .fields
            .iter()
            .enumerate()
            .map(|(field_n, field)| match field.ident {
                Some(ref i) => quote! {
                    self.#i.on_listener_event(event);
                },
                None => quote! {
                    self.#field_n.on_listener_event(event);
                },
            })
    };

    let final_quote = quote! {
        #network_behavior_token
        impl #impl_generics #network_incoming_behavior_to_impl for #name #ty_generics
        #where_clause
        {
            fn handle_pending_connection(
                &mut self,
                id: #connection_id,
                local_addr: &#url,
                remote_addr: &#url
            ) -> Result<(), #connection_denied> {
                #(#handle_pending_inbound_connection_stmts)*
                Ok(())
            }

            fn handle_established_connection(
                &mut self,
                id: #connection_id,
                peer_id: #peer_id,
                local_addr: &#url,
                remote_addr: &#url
            ) -> Result<Self::ConnectionHandler, #connection_denied> {
                Ok(#handle_established_inbound_connection)
            }

            fn on_connection_established(
                &mut self,
                id: #connection_id,
                peer_id: #peer_id,
                local_addr: &#url,
                remote_addr: &#url,
            ) {
                #(#on_connection_established_stmts)*
            }

            fn on_connection_closed(
                &mut self,
                id: #connection_id,
                peer_id: #peer_id,
                local_addr: &#url,
                remote_addr: &#url,
                reason: Option<&#connection_error>,
            ) {
                #(#on_connection_closed_stmts)*
            }

            fn on_listen_failure(
                &mut self,
                id: #connection_id,
                peer_id: Option<#peer_id>,
                local_addr: &#url,
                remote_addr: &#url,
                error: &#listen_error,
            ) {
                #(#on_listen_failure_stmts)*
            }

            fn on_listener_event(&mut self, event: #listener_event<'_>) {
                #(#on_listener_event_stmts)*
            }
        }

    };

    return Ok(final_quote.into());
}

fn build_outgoing_struct(ast: &DeriveInput, data_struct: &DataStruct) -> syn::Result<TokenStream> {
    let common_parsed = parse_common_token_stream(ast)?;
    // 结构体名称
    let name = &ast.ident;
    // ty_generics: 泛型参数, where_clause: where 子句
    let (_, ty_generics, _) = ast.generics.split_for_impl();

    let (network_behavior_token, out_event_from_clauses) =
        build_network_behavior_impl(ast, data_struct, &common_parsed);

    let CommonParsed {
        prelude:
            PreludeTokenStream {
                url,
                peer_id,
                connection_id,
                connection_denied,
                network_outgoing_behavior_to_impl,
                connection_handler,
                dial_error,
                connection_error,
                dial_opts,
                impl_generics,
                ..
            },
        ..
    } = &common_parsed;

    let where_clause = where_clause_token(
        ast,
        data_struct,
        out_event_from_clauses,
        network_outgoing_behavior_to_impl,
    );

    let handle_pending_outbound_connection = {
        let extend_stmts =
            data_struct
                .fields
                .iter()
                .enumerate()
                .map(|(field_n, field)| {
                    match field.ident {
                        Some(ref i) => quote! {
                            if let Some(addr) = #network_outgoing_behavior_to_impl::handle_pending_connection(&mut self.#i, id, maybe_peer, &maybe_addr)? {
                                maybe_addr = Some(addr);
                            }
                        },
                        None => quote! {
                            if let Some(addr) = #network_outgoing_behavior_to_impl::handle_pending_connection(&mut self.#field_n, id, maybe_peer, &maybe_addr)? {
                                maybe_addr = Some(addr);
                            }
                        }
                    }
                });

        quote! {
            let mut maybe_addr = maybe_addr.clone();
            #(#extend_stmts)*
            Ok(maybe_addr)
        }
    };

    let handle_established_outbound_connection = {
        let mut out_handler = None;

        for (field_n, field) in data_struct.fields.iter().enumerate() {
            let field_name = match field.ident {
                Some(ref i) => quote! { self.#i },
                None => quote! { self.#field_n },
            };

            let builder = quote! {
                #field_name.handle_established_connection(id, peer_id, addr)?
            };

            match out_handler {
                Some(h) => out_handler = Some(quote! { #connection_handler::select(#h, #builder) }),
                ref mut h @ None => *h = Some(builder),
            }
        }
        out_handler.unwrap_or(quote! {()})
    };

    // 生成 on_connection_established
    let on_connection_established_stmts = data_struct.fields.iter().enumerate().map(
        |(field_n, field)| {
            match field.ident {
                Some(ref i) => quote! {
                    #network_outgoing_behavior_to_impl::on_connection_established(&mut self.#i, id, peer_id, addr);
                },
                None => quote! {
                    #network_outgoing_behavior_to_impl::on_connection_established(&mut self.#field_n, id, peer_id, addr);
                },
            }
        },
    );

    // 生成 on_connection_closed
    let on_connection_closed_stmts = data_struct.fields.iter().enumerate().map(
        |(field_n, field)| {
            match field.ident {
                Some(ref i) => quote! {
                    #network_outgoing_behavior_to_impl::on_connection_closed(&mut self.#i, id, peer_id, addr, reason);
                },
                None => quote! {
                    #network_outgoing_behavior_to_impl::on_connection_closed(&mut self.#field_n, id, peer_id, addr, reason);
                },
            }
        },
    );

    // 生成 on_dial_failure
    let on_dial_failure_stmts = data_struct.fields.iter().enumerate().map(
        |(field_n, field)| {
            match field.ident {
                Some(ref i) => quote! {
                    #network_outgoing_behavior_to_impl::on_dial_failure(&mut self.#i, id, maybe_peer, maybe_addr, error);
                },
                None => quote! {
                    #network_outgoing_behavior_to_impl::on_dial_failure(&mut self.#field_n, id, maybe_peer, maybe_addr, error);
                },
            }
        },
    );

    let poll_stmts = data_struct.fields.iter().enumerate().map(|(_, field)| {
        let field = field
            .ident
            .clone()
            .expect("Fields of NetworkBehavior implementation to be named.");
        quote! {
            match #network_outgoing_behavior_to_impl::poll_dial(&mut self.#field, cx) {
                std::task::Poll::Ready(opts) => return std::task::Poll::Ready(opts),
                std::task::Poll::Pending => {},
            }
        }
    });

    let final_quote = quote! {
        #network_behavior_token
        impl #impl_generics #network_outgoing_behavior_to_impl for #name #ty_generics
        #where_clause
        {
            fn handle_pending_connection(
                &mut self,
                id: #connection_id,
                maybe_peer: Option<#peer_id>,
                maybe_addr: &Option<#url>,
            ) -> Result<Option<#url>, #connection_denied> {
                #handle_pending_outbound_connection
            }

            fn handle_established_connection(
                &mut self,
                id: #connection_id,
                peer_id: #peer_id,
                addr: &#url,
            ) -> Result<Self::ConnectionHandler, #connection_denied> {
                Ok(#handle_established_outbound_connection)
            }

            /// 连接处理器事件处理
            fn on_connection_established(
                &mut self,
                id: #connection_id,
                peer_id: #peer_id,
                addr: &#url
            ) {
                #(#on_connection_established_stmts)*
            }

            fn on_connection_closed(
                &mut self,
                id: #connection_id,
                peer_id: #peer_id,
                addr: &#url,
                reason: Option<&#connection_error>,
            ) {
                #(#on_connection_closed_stmts)*
            }

            fn on_dial_failure(
                &mut self,
                id: #connection_id,
                maybe_peer: Option<#peer_id>,
                maybe_addr: Option<&#url>,
                error: &#dial_error,
            ) {
                #(#on_dial_failure_stmts)*
            }

            fn poll_dial(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<#dial_opts> {
                #(#poll_stmts)*
                std::task::Poll::Pending
            }
        }
    };
    return Ok(final_quote.into());
}

struct BehaviorAttributes {
    // 引入的预定义模块路径
    prelude_path: syn::Path,
    // 用户指定的事件类型
    user_specified_out_event: Option<syn::Type>,
}

// 解析结构体中的#[behavior]属性
fn parse_attributes(ast: &DeriveInput) -> syn::Result<BehaviorAttributes> {
    // 默认参数
    let mut attributes = BehaviorAttributes {
        prelude_path: syn::parse_quote! { ::volans::swarm::derive_prelude },
        user_specified_out_event: None,
    };

    // 查找并解析 #[behavior] 属性
    for attr in ast
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("behavior"))
    {
        // #[behavior(prelude=path, to_swarm=Type, out_event=Type)]
        let nested = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in nested {
            if meta.path().is_ident("prelude") {
                let value = meta.require_name_value()?.value.require_str_lit()?;
                attributes.prelude_path = syn::parse_str(&value)?;
            } else if meta.path().is_ident("to_swarm") || meta.path().is_ident("out_event") {
                let value = meta.require_name_value()?.value.require_str_lit()?;
                attributes.user_specified_out_event = Some(syn::parse_str(&value)?);
            }
        }
    }

    Ok(attributes)
}
