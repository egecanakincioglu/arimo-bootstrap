// ─────────────────────────────────────────────────────────────────────────────
// Arimo Lang — CodeGen (LLVM / inkwell)
// Hedef: .arm → LLVM IR → native binary
// ─────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::path::Path;

use inkwell::context::Context;
use inkwell::builder::Builder;
use inkwell::module::Module;
use inkwell::values::{
    BasicValueEnum, FunctionValue, PointerValue, BasicValue,
};
use inkwell::types::{BasicTypeEnum, BasicType};
use inkwell::AddressSpace;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode,
    Target, TargetMachine,
};
use inkwell::OptimizationLevel;

use crate::ast::*;

// ─────────────────────────────────────────────────────────────────────────────
// Hata tipi
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct CodeGenError {
    pub message: String,
}

impl CodeGenError {
    fn new(msg: impl Into<String>) -> Self {
        CodeGenError { message: msg.into() }
    }
}

impl std::fmt::Display for CodeGenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "codegen error — {}", self.message)
    }
}

type CgResult<T> = Result<T, CodeGenError>;

// ─────────────────────────────────────────────────────────────────────────────
// Değişken bilgisi (stack slot)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct VarSlot<'ctx> {
    ptr        : PointerValue<'ctx>,
    ty         : BasicTypeEnum<'ctx>,
    class_name : Option<String>,   // class instance veya "__List"/"__HashMap"/"__Pair"
    elem_class : Option<String>,   // List<T> için T'nin class adı
}

// ─────────────────────────────────────────────────────────────────────────────
// CodeGen yapısı
// ─────────────────────────────────────────────────────────────────────────────

pub struct CodeGen<'ctx> {
    ctx     : &'ctx Context,
    module  : Module<'ctx>,
    builder : Builder<'ctx>,
    // Yerel değişkenler: kapsam yığını
    scopes  : Vec<HashMap<String, VarSlot<'ctx>>>,
    // Tanımlı fonksiyonlar
    fns     : HashMap<String, FunctionValue<'ctx>>,
    // Geçerli fonksiyon
    cur_fn  : Option<FunctionValue<'ctx>>,
    // Class struct tipleri: class adı → LLVM struct type
    struct_types  : HashMap<String, inkwell::types::StructType<'ctx>>,
    // Field indeksleri: class adı → field adı → indeks
    field_indices : HashMap<String, HashMap<String, u32>>,
    // Geçerli class (this erişimi için)
    cur_class : Option<String>,
    // Enum variant değerleri: enum adı → variant adı → i32 değeri
    enum_variants : HashMap<String, HashMap<String, u32>>,
    // Static field global'ları: "ClassName.fieldName" → global pointer
    static_fields : HashMap<String, inkwell::values::GlobalValue<'ctx>>,
    // Field Arimo tipleri: class adı → field adı → tür adı (method dispatch için)
    field_arimo_types : HashMap<String, HashMap<String, String>>,
    // Lambda fonksiyonları için benzersiz sayaç
    lambda_counter    : usize,
}

impl<'ctx> CodeGen<'ctx> {
    pub fn new(ctx: &'ctx Context, module_name: &str) -> Self {
        let module  = ctx.create_module(module_name);
        let builder = ctx.create_builder();
        CodeGen {
            ctx,
            module,
            builder,
            scopes        : Vec::new(),
            fns           : HashMap::new(),
            cur_fn        : None,
            struct_types  : HashMap::new(),
            field_indices : HashMap::new(),
            cur_class     : None,
            enum_variants : HashMap::new(),
            static_fields         : HashMap::new(),
            field_arimo_types     : HashMap::new(),
            lambda_counter        : 0,
        }
    }

    // ── Kapsam yönetimi ──────────────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define_var(&mut self, name: &str, ptr: PointerValue<'ctx>, ty: BasicTypeEnum<'ctx>) {
        self.define_var_with_class(name, ptr, ty, None);
    }

    fn define_var_with_class(
        &mut self,
        name       : &str,
        ptr        : PointerValue<'ctx>,
        ty         : BasicTypeEnum<'ctx>,
        class_name : Option<String>,
    ) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarSlot { ptr, ty, class_name, elem_class: None });
        }
    }

    fn define_collection_var(
        &mut self,
        name       : &str,
        ptr        : PointerValue<'ctx>,
        ty         : BasicTypeEnum<'ctx>,
        class_name : Option<String>,
        elem_class : Option<String>,
    ) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarSlot { ptr, ty, class_name, elem_class });
        }
    }

    fn infer_elem_class(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => {
                for scope in self.scopes.iter().rev() {
                    if let Some(slot) = scope.get(name.as_str()) {
                        if slot.elem_class.is_some() {
                            return slot.elem_class.clone();
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&VarSlot<'ctx>> {
        for scope in self.scopes.iter().rev() {
            if let Some(slot) = scope.get(name) {
                return Some(slot);
            }
        }
        None
    }

    // ── Tip çevirisi: Arimo → LLVM ───────────────────────────────────────────

    fn llvm_type(&self, ty: &Type) -> Option<BasicTypeEnum<'ctx>> {
        match ty {
            Type::Integer         => Some(self.ctx.i64_type().into()),
            Type::Float           => Some(self.ctx.f64_type().into()),
            Type::Boolean         => Some(self.ctx.bool_type().into()),
            Type::Str             => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::Void            => None,
            Type::NoReturn        => None,
            Type::U8  | Type::I8  => Some(self.ctx.i8_type().into()),
            Type::U16 | Type::I16 => Some(self.ctx.i16_type().into()),
            Type::U32 | Type::I32 => Some(self.ctx.i32_type().into()),
            Type::U64 | Type::I64 => Some(self.ctx.i64_type().into()),
            Type::RawPtr(_)       => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            // Koleksiyon tipleri — opaque pointer
            Type::List(_) | Type::Map(..) | Type::HashMap(..) | Type::TreeMap(..)
            | Type::Pair(..) | Type::Slice(_) | Type::Array(..)
                => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::Generic(_, _)   => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::Nullable(inner) => self.llvm_type(inner),
            // Enum tipler → i32 integer
            Type::Named(n) if self.is_enum(n) => Some(self.ctx.i32_type().into()),
            Type::Named(_) => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::FnPtr(params, ret) => {
                let ret_llvm = self.llvm_type(ret.as_ref());
                let params_llvm: Vec<inkwell::types::BasicMetadataTypeEnum> = params.iter()
                    .filter_map(|p| self.llvm_type(p))
                    .map(|t| t.into())
                    .collect();
                let fn_ty = match ret_llvm {
                    Some(r) => r.fn_type(&params_llvm, false),
                    None    => self.ctx.void_type().fn_type(&params_llvm, false),
                };
                Some(fn_ty.ptr_type(AddressSpace::default()).into())
            }
            _ => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
        }
    }

    // ── Modülü işle ──────────────────────────────────────────────────────────

    pub fn compile_module(&mut self, module: &crate::ast::Module) -> CgResult<()> {
        // Extern fonksiyonları kayıt et
        for item in &module.items {
            if let Item::Extern(ext) = item {
                self.declare_extern_block(ext)?;
            }
        }

        // printf + malloc + koleksiyon runtime her zaman lazım
        self.declare_printf();
        self.declare_malloc();
        self.declare_collection_runtime();

        // Geçiş 0: enum variant'larını kayıt et
        for item in &module.items {
            if let Item::Enum(e) = item {
                self.register_enum(e);
            }
        }
        // Geçiş 1: struct tiplerini kayıt et (field layout)
        for item in &module.items {
            if let Item::Class(c) = item {
                self.register_class_struct(c);
            }
        }
        // Geçiş 2: tüm fonksiyon imzaları (static + instance)
        for item in &module.items {
            if let Item::Class(c) = item {
                self.register_class_methods(c)?;
            }
        }
        // Geçiş 2.5: static field başlangıç değerlerini ayarla
        for item in &module.items {
            if let Item::Class(c) = item {
                self.init_static_fields(c)?;
            }
        }
        // Geçiş 3: gövdeleri üret
        for item in &module.items {
            match item {
                Item::Class(c) => self.compile_class(c)?,
                Item::Enum(e)  => self.compile_enum(e)?,
                _              => {}
            }
        }
        Ok(())
    }

    // ── malloc extern tanımı ─────────────────────────────────────────────────

    fn declare_malloc(&mut self) {
        if self.module.get_function("malloc").is_some() { return; }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let malloc_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
        self.module.add_function("malloc", malloc_ty, None);
    }

    // ── Class struct tipi kayıt ──────────────────────────────────────────────

    fn register_class_struct(&mut self, c: &ClassDecl) {
        // Inheritance: parent field'larını da struct'a ekle
        let mut all_field_types: Vec<BasicTypeEnum<'ctx>> = Vec::new();
        let mut idx_map: HashMap<String, u32> = HashMap::new();
        let mut idx = 0u32;

        // Parent struct field'ları önce gelir
        if let Some(parent_name) = &c.extends {
            if let Some(parent_fields) = self.field_indices.get(parent_name).cloned() {
                if let Some(parent_struct) = self.struct_types.get(parent_name).copied() {
                    for fi in 0..parent_struct.count_fields() {
                        if let Some(ft) = parent_struct.get_field_type_at_index(fi) {
                            all_field_types.push(ft);
                        }
                    }
                    for (fname, fidx) in &parent_fields {
                        idx_map.insert(fname.clone(), *fidx);
                        idx = idx.max(*fidx + 1);
                    }
                }
            }
        }

        // Kendi instance field'ları
        for f in c.fields.iter().filter(|f| !f.static_) {
            if let Some(ft) = self.llvm_type(&f.ty) {
                all_field_types.push(ft);
                idx_map.insert(f.name.clone(), idx);
                idx += 1;
            }
        }

        let struct_ty = self.ctx.struct_type(&all_field_types, false);
        self.struct_types.insert(c.name.clone(), struct_ty);
        self.field_indices.insert(c.name.clone(), idx_map);

        // Arimo tip bilgisini kaydet (dispatch için)
        let mut arimo_types: HashMap<String, String> = HashMap::new();
        // Önce parent'ın tiplerini kopyala
        if let Some(parent_name) = &c.extends {
            if let Some(parent_arimo) = self.field_arimo_types.get(parent_name).cloned() {
                arimo_types.extend(parent_arimo);
            }
        }
        for f in c.fields.iter().filter(|f| !f.static_) {
            let type_name = match &f.ty {
                Type::Named(n) => n.clone(),
                Type::Nullable(inner) => match inner.as_ref() {
                    Type::Named(n) => n.clone(),
                    other => format!("{:?}", other),
                },
                other => format!("{:?}", other),
            };
            arimo_types.insert(f.name.clone(), type_name);
        }
        self.field_arimo_types.insert(c.name.clone(), arimo_types);

        // Static field'lar → LLVM global değişkenler
        for f in c.fields.iter().filter(|f| f.static_) {
            let global_name = format!("{}_{}", c.name, f.name);
            if self.module.get_global(&global_name).is_some() { continue; }

            if let Some(llvm_ty) = self.llvm_type(&f.ty) {
                let zero = self.make_zero_value(llvm_ty);
                let global = self.module.add_global(llvm_ty, None, &global_name);
                global.set_initializer(&zero);
                global.set_linkage(inkwell::module::Linkage::Internal);
                self.static_fields.insert(global_name, global);
            }
        }
    }

    fn make_zero_value(&self, ty: BasicTypeEnum<'ctx>) -> inkwell::values::BasicValueEnum<'ctx> {
        match ty {
            BasicTypeEnum::IntType(t)     => t.const_int(0, false).into(),
            BasicTypeEnum::FloatType(t)   => t.const_float(0.0).into(),
            BasicTypeEnum::PointerType(t) => t.const_null().into(),
            _                             => self.ctx.i64_type().const_int(0, false).into(),
        }
    }

    // ── Enum kayıt ───────────────────────────────────────────────────────────

    // Static field başlangıç değerleri — compile zamanında sabit ise global'e yaz
    fn init_static_fields(&mut self, c: &ClassDecl) -> CgResult<()> {
        for f in c.fields.iter().filter(|f| f.static_) {
            let global_name = format!("{}_{}", c.name, f.name);
            if let Some(gv) = self.module.get_global(&global_name) {
                if let Some(init_expr) = &f.value {
                    // Sadece compile-time sabit değerleri direkt ata
                    let const_val: Option<inkwell::values::BasicValueEnum> = match init_expr {
                        Expr::IntLit(n)   => Some(self.ctx.i64_type().const_int(*n as u64, *n < 0).into()),
                        Expr::FloatLit(f) => Some(self.ctx.f64_type().const_float(*f).into()),
                        Expr::BoolLit(b)  => Some(self.ctx.bool_type().const_int(*b as u64, false).into()),
                        Expr::StrLit(s)   => {
                            // String static field: global char array olarak sakla
                            let bytes = s.as_bytes();
                            let char_arr = self.ctx.const_string(bytes, true);
                            let str_global = self.module.add_global(
                                char_arr.get_type(), None,
                                &format!("{}_strdata", global_name)
                            );
                            str_global.set_initializer(&char_arr);
                            str_global.set_linkage(inkwell::module::Linkage::Internal);
                            Some(str_global.as_pointer_value().into())
                        }
                        _ => None,
                    };
                    if let Some(cv) = const_val {
                        gv.set_initializer(&cv);
                    }
                }
            }
        }
        Ok(())
    }

    fn register_enum(&mut self, e: &EnumDecl) {
        let mut variants = HashMap::new();
        for (i, v) in e.variants.iter().enumerate() {
            variants.insert(v.name.clone(), i as u32);
        }
        self.enum_variants.insert(e.name.clone(), variants);

        // Enum metodları kayıt et (i32 this parametreli)
        let i32_ty = self.ctx.i32_type();
        for m in &e.methods {
            let fn_name = format!("{}_{}", e.name, m.name);
            if self.module.get_function(&fn_name).is_some() { continue; }

            // this = i32 (enum değeri)
            let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> =
                vec![i32_ty.into()];
            for p in &m.params {
                if let Some(t) = self.llvm_type(&p.ty) {
                    param_types.push(t.into());
                }
            }

            let fn_val = match &m.return_ty {
                Some(ret_ty) => match self.llvm_type(ret_ty) {
                    Some(rt) => self.module.add_function(&fn_name, rt.fn_type(&param_types, false), None),
                    None     => self.module.add_function(&fn_name, self.ctx.void_type().fn_type(&param_types, false), None),
                },
                None => self.module.add_function(&fn_name, self.ctx.void_type().fn_type(&param_types, false), None),
            };
            self.fns.insert(fn_name, fn_val);
        }
    }

    fn is_enum(&self, name: &str) -> bool {
        self.enum_variants.contains_key(name)
    }

    fn enum_variant_value(&self, enum_name: &str, variant: &str) -> Option<u32> {
        self.enum_variants.get(enum_name)?.get(variant).copied()
    }

    // ── printf extern tanımı ─────────────────────────────────────────────────

    fn declare_printf(&mut self) {
        if self.module.get_function("printf").is_some() { return; }
        let i8_ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32_ty = self.ctx.i32_type();
        let printf_ty = i32_ty.fn_type(&[i8_ptr.into()], true);
        self.module.add_function("printf", printf_ty, None);
    }

    // ── Extern "C" blokları ──────────────────────────────────────────────────

    fn declare_extern_block(&mut self, block: &ExternBlock) -> CgResult<()> {
        for decl in &block.decls {
            if self.module.get_function(&decl.name).is_some() { continue; }
            let params: Vec<inkwell::types::BasicMetadataTypeEnum> = decl.params.iter()
                .filter_map(|p| self.llvm_type(&p.ty).map(|t| t.into()))
                .collect();
            let fn_ty = match &decl.return_ty {
                Some(ret) => match self.llvm_type(ret) {
                    Some(rt) => rt.fn_type(&params, decl.variadic),
                    None     => self.ctx.void_type().fn_type(&params, decl.variadic),
                },
                None => self.ctx.void_type().fn_type(&params, decl.variadic),
            };
            self.module.add_function(&decl.name, fn_ty, None);
        }
        Ok(())
    }

    // ── Class metodlarını kayıt et (imza) ────────────────────────────────────

    fn register_class_methods(&mut self, c: &ClassDecl) -> CgResult<()> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        // Constructor: ClassName_new(fields...) → ptr
        if c.constructor.is_some() {
            let ctor_name = format!("{}_new", c.name);
            if self.module.get_function(&ctor_name).is_none() {
                let ctor = c.constructor.as_ref().unwrap();
                let params: Vec<inkwell::types::BasicMetadataTypeEnum> = ctor.params.iter()
                    .filter_map(|p| self.llvm_type(&p.ty).map(|t| t.into()))
                    .collect();
                let fn_ty = ptr_ty.fn_type(&params, false);
                let fn_val = self.module.add_function(&ctor_name, fn_ty, None);
                self.fns.insert(ctor_name, fn_val);
            }
        }

        // Metodlar
        for m in &c.methods {
            let fn_name = format!("{}_{}", c.name, m.name);
            if self.module.get_function(&fn_name).is_some() { continue; }

            let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::new();
            if !m.static_ {
                // this pointer
                param_types.push(ptr_ty.into());
            }
            for p in &m.params {
                if let Some(t) = self.llvm_type(&p.ty) {
                    param_types.push(t.into());
                }
            }

            let fn_val = match &m.return_ty {
                Some(ret_ty) => match self.llvm_type(ret_ty) {
                    Some(rt) => {
                        let fn_ty = rt.fn_type(&param_types, false);
                        self.module.add_function(&fn_name, fn_ty, None)
                    }
                    None => {
                        let fn_ty = self.ctx.void_type().fn_type(&param_types, false);
                        self.module.add_function(&fn_name, fn_ty, None)
                    }
                },
                None => {
                    let fn_ty = self.ctx.void_type().fn_type(&param_types, false);
                    self.module.add_function(&fn_name, fn_ty, None)
                }
            };
            self.fns.insert(fn_name, fn_val);
        }
        Ok(())
    }

    // ── Class gövdelerini derle ───────────────────────────────────────────────

    // ── Enum gövdeleri ───────────────────────────────────────────────────────

    fn compile_enum(&mut self, e: &EnumDecl) -> CgResult<()> {
        self.cur_class = Some(e.name.clone());
        for m in &e.methods.clone() {
            if m.body.is_none() { continue; }
            self.compile_enum_method(&e.name.clone(), m)?;
        }
        self.cur_class = None;
        Ok(())
    }

    fn compile_enum_method(&mut self, enum_name: &str, m: &Method) -> CgResult<()> {
        let fn_name = format!("{}_{}", enum_name, m.name);
        let fn_val = match self.fns.get(&fn_name).copied()
            .or_else(|| self.module.get_function(&fn_name))
        {
            Some(f) => f,
            None    => return Ok(()),
        };

        let body = match m.body.as_ref() { Some(b) => b.clone(), None => return Ok(()) };

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        self.cur_fn = Some(fn_val);
        self.push_scope();

        // this = i32 (enum değeri), 0. parametre
        if let Some(this_val) = fn_val.get_nth_param(0) {
            let i32_ty = self.ctx.i32_type();
            let this_alloca = self.builder.build_alloca(i32_ty, "this")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_store(this_alloca, this_val)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.define_var("this", this_alloca, i32_ty.into());
        }

        // Diğer parametreler
        for (i, p) in m.params.iter().enumerate() {
            if let Some(llvm_ty) = self.llvm_type(&p.ty) {
                if let Some(pv) = fn_val.get_nth_param((i + 1) as u32) {
                    let alloca = self.builder.build_alloca(llvm_ty, &p.name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(alloca, pv)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.define_var(&p.name, alloca, llvm_ty);
                }
            }
        }

        let mut returned = false;
        for stmt in &body {
            if self.compile_stmt(stmt)? { returned = true; break; }
        }
        if !returned {
            self.builder.build_return(None)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.pop_scope();
        self.cur_fn = None;

        if fn_val.verify(true) { Ok(()) }
        else { Err(CodeGenError::new(format!("enum method verify failed: {}_{}", enum_name, m.name))) }
    }

    fn compile_class(&mut self, c: &ClassDecl) -> CgResult<()> {
        self.cur_class = Some(c.name.clone());

        // Constructor gövdesi
        if let Some(ctor) = &c.constructor.clone() {
            self.compile_constructor(c, ctor)?;
        }

        // Metod gövdeleri
        for m in &c.methods.clone() {
            if m.body.is_none() { continue; }
            self.compile_method(c, m)?;
        }

        self.cur_class = None;
        Ok(())
    }

    // ── Constructor gövdesi ──────────────────────────────────────────────────

    fn compile_constructor(&mut self, c: &ClassDecl, ctor: &Constructor) -> CgResult<()> {
        let ctor_name = format!("{}_new", c.name);
        let fn_val = match self.fns.get(&ctor_name).copied()
            .or_else(|| self.module.get_function(&ctor_name))
        {
            Some(f) => f,
            None    => return Ok(()),
        };

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        self.cur_fn = Some(fn_val);
        self.push_scope();

        // malloc ile bellek ayır
        let malloc = self.module.get_function("malloc")
            .ok_or_else(|| CodeGenError::new("malloc not declared"))?;
        let struct_ty = self.struct_types.get(&c.name).copied()
            .ok_or_else(|| CodeGenError::new(format!("struct type not found: {}", c.name)))?;
        let size = struct_ty.size_of()
            .ok_or_else(|| CodeGenError::new("struct has no size"))?;
        let obj_ptr = self.builder
            .build_call(malloc, &[size.into()], "obj")
            .map_err(|e| CodeGenError::new(e.to_string()))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodeGenError::new("malloc returned void"))?;

        // 'this' alloca'sına kaydet
        let this_alloca = self.builder
            .build_alloca(self.ctx.ptr_type(AddressSpace::default()), "this")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(this_alloca, obj_ptr)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.define_var("this", this_alloca,
            self.ctx.ptr_type(AddressSpace::default()).into());

        // Parametre alloca'ları
        for (i, p) in ctor.params.iter().enumerate() {
            if let Some(llvm_ty) = self.llvm_type(&p.ty) {
                if let Some(pv) = fn_val.get_nth_param(i as u32) {
                    let alloca = self.builder.build_alloca(llvm_ty, &p.name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(alloca, pv)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.define_var(&p.name, alloca, llvm_ty);
                }
            }
        }

        // Constructor body
        for stmt in &ctor.body.clone() {
            if self.compile_stmt(stmt)? { break; }
        }

        // this pointer'ı döndür
        let ptr_ty: BasicTypeEnum<'ctx> = self.ctx.ptr_type(AddressSpace::default()).into();
        let this_ptr = self.builder
            .build_load(ptr_ty, this_alloca, "ret_this")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_return(Some(&this_ptr))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.pop_scope();
        self.cur_fn = None;

        if fn_val.verify(true) { Ok(()) }
        else { Err(CodeGenError::new(format!("constructor verification failed: {}", c.name))) }
    }

    // ── Metod gövdesi ─────────────────────────────────────────────────────────

    fn compile_method(&mut self, c: &ClassDecl, m: &Method) -> CgResult<()> {
        let fn_name = format!("{}_{}", c.name, m.name);

        // main() → LLVM main fonksiyonu
        let is_entry = m.name == "main" && m.static_;

        let fn_val = if is_entry {
            // main fonksiyonunu özel olarak tanımla: i32 main()
            let main_ty = self.ctx.i32_type().fn_type(&[], false);
            if let Some(existing) = self.module.get_function("main") {
                existing
            } else {
                self.module.add_function("main", main_ty, None)
            }
        } else {
            match self.fns.get(&fn_name).copied() {
                Some(f) => f,
                None => return Err(CodeGenError::new(format!("function not registered: {}", fn_name))),
            }
        };

        let body = m.body.as_ref().unwrap();

        let entry_block = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry_block);

        self.cur_fn = Some(fn_val);
        self.push_scope();

        // Instance metod: this pointer'ı kapsama ekle
        if !m.static_ {
            if let Some(this_val) = fn_val.get_nth_param(0) {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let this_alloca = self.builder.build_alloca(ptr_ty, "this")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(this_alloca, this_val)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.define_var("this", this_alloca, ptr_ty.into());
            }
        }

        // Parametreleri kapsama ekle
        let param_offset = if m.static_ { 0 } else { 1 };
        for (i, p) in m.params.iter().enumerate() {
            if let Some(llvm_ty) = self.llvm_type(&p.ty) {
                let param_val = fn_val.get_nth_param((i + param_offset) as u32);
                if let Some(pv) = param_val {
                    let alloca = self.builder.build_alloca(llvm_ty, &p.name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(alloca, pv)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.define_var(&p.name, alloca, llvm_ty);
                }
            }
        }

        // Statement'ları derle
        let mut returned = false;
        for stmt in body {
            if self.compile_stmt(stmt)? {
                returned = true;
                break;
            }
        }

        // Return yoksa otomatik ekle
        if !returned {
            if is_entry {
                let zero = self.ctx.i32_type().const_int(0, false);
                self.builder.build_return(Some(&zero))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }
        }

        self.pop_scope();
        self.cur_fn = None;

        // LLVM doğrulaması
        if fn_val.verify(true) {
            Ok(())
        } else {
            Err(CodeGenError::new(format!("LLVM function verification failed: {}", fn_name)))
        }
    }

    // ── Statement'lar → LLVM IR ───────────────────────────────────────────────
    // Dönüş değeri: true = bu statement bir return içeriyor

    fn compile_stmt(&mut self, stmt: &Stmt) -> CgResult<bool> {
        match stmt {
            Stmt::Return(expr) => {
                match expr {
                    None => {
                        // main() için i32 0 döndür
                        let cur = self.cur_fn.unwrap();
                        if cur.get_type().get_return_type().is_some() {
                            let zero = self.ctx.i32_type().const_int(0, false);
                            self.builder.build_return(Some(&zero))
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        } else {
                            self.builder.build_return(None)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                    }
                    Some(e) => {
                        let val = self.compile_expr(e)?;
                        match val {
                            Some(v) => {
                                self.builder.build_return(Some(&v))
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                            }
                            None => {
                                self.builder.build_return(None)
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                            }
                        }
                    }
                }
                Ok(true)
            }

            Stmt::VarDecl { ty, name, value, .. } => {
                // Koleksiyon tipi mi?
                let (class_name, elem_class) = match ty {
                    Type::Named(n) if self.struct_types.contains_key(n.as_str()) => {
                        (Some(n.clone()), None)
                    }
                    Type::List(inner) => {
                        let ec = match inner.as_ref() {
                            Type::Named(n) => Some(n.clone()),
                            _ => None,
                        };
                        (Some("__List".to_string()), ec)
                    }
                    Type::HashMap(..) | Type::Map(..) | Type::TreeMap(..) => {
                        (Some("__HashMap".to_string()), None)
                    }
                    Type::Pair(..) => (Some("__Pair".to_string()), None),
                    _ => (None, None),
                };

                // Enum tip → i32, class/collection instance → pointer, diğerleri → direkt tip
                let llvm_ty = if class_name.is_some() {
                    Some(BasicTypeEnum::from(self.ctx.ptr_type(AddressSpace::default())))
                } else if let Type::Named(n) = ty {
                    if self.is_enum(n) {
                        Some(BasicTypeEnum::from(self.ctx.i32_type()))
                    } else {
                        self.llvm_type(ty)
                    }
                } else {
                    self.llvm_type(ty)
                };

                if let Some(llvm_ty) = llvm_ty {
                    let alloca = self.builder.build_alloca(llvm_ty, name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    if let Some(init_expr) = value {
                        if let Some(val) = self.compile_expr(init_expr)? {
                            let coerced = self.coerce_value(val, llvm_ty)?;
                            self.builder.build_store(alloca, coerced)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                    }
                    if elem_class.is_some() || matches!(class_name.as_deref(), Some("__List" | "__HashMap" | "__Pair")) {
                        self.define_collection_var(name, alloca, llvm_ty, class_name, elem_class);
                    } else {
                        self.define_var_with_class(name, alloca, llvm_ty, class_name);
                    }
                }
                Ok(false)
            }

            Stmt::ExprStmt(e) => {
                self.compile_expr(e)?;
                Ok(false)
            }

            Stmt::If { hint: _, cond, then, else_if, else_ } => {
                self.compile_if(cond, then, else_if, else_)
            }

            Stmt::While { cond, body } => {
                self.compile_while(cond, body)
            }

            Stmt::For { init, cond, step, body } => {
                self.compile_for(init, cond, step, body)
            }

            Stmt::ForEach { ty, name, iter, body } => {
                self.compile_for_each(ty, name, iter, body)
            }

            Stmt::Block(stmts) => {
                self.push_scope();
                let mut returned = false;
                for s in stmts {
                    if self.compile_stmt(s)? {
                        returned = true;
                        break;
                    }
                }
                self.pop_scope();
                Ok(returned)
            }

            Stmt::TryCatch { try_body, catches: _, finally_body } => {
                // Basit implementasyon: try bloğunu doğrudan çalıştır
                let mut ret = false;
                self.push_scope();
                for s in try_body {
                    if self.compile_stmt(s)? { ret = true; break; }
                }
                self.pop_scope();
                if let Some(fin) = finally_body {
                    self.push_scope();
                    for s in fin {
                        if self.compile_stmt(s)? { break; }
                    }
                    self.pop_scope();
                }
                Ok(ret)
            }

            Stmt::Throw(_) => {
                // Şimdilik: abort() çağır
                self.builder.build_unreachable()
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(true)
            }

            Stmt::Switch { expr, cases } => {
                self.compile_switch(expr, cases)
            }

            Stmt::Asm(_) | Stmt::Defer(_) => Ok(false),
            Stmt::Break | Stmt::Continue   => Ok(false),
        }
    }

    // ── if / else ────────────────────────────────────────────────────────────

    fn compile_if(
        &mut self,
        cond    : &Expr,
        then    : &[Stmt],
        else_if : &[(Expr, Vec<Stmt>)],
        else_   : &Option<Vec<Stmt>>,
    ) -> CgResult<bool> {
        let cur_fn = self.cur_fn.unwrap();
        let cond_val = self.compile_expr(cond)?
            .ok_or_else(|| CodeGenError::new("if condition has no value"))?;
        let cond_bool = self.to_bool(cond_val)?;

        let then_bb  = self.ctx.append_basic_block(cur_fn, "if.then");
        let merge_bb = self.ctx.append_basic_block(cur_fn, "if.merge");
        let else_bb  = if else_.is_some() || !else_if.is_empty() {
            self.ctx.append_basic_block(cur_fn, "if.else")
        } else {
            merge_bb
        };

        self.builder.build_conditional_branch(cond_bool, then_bb, else_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // then bloğu
        self.builder.position_at_end(then_bb);
        self.push_scope();
        let mut then_returned = false;
        for s in then {
            if self.compile_stmt(s)? { then_returned = true; break; }
        }
        self.pop_scope();
        if !then_returned {
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        // else / else-if bloğu
        let mut else_returned = false;
        if else_bb != merge_bb {
            self.builder.position_at_end(else_bb);
            self.push_scope();
            if let Some(eb) = else_ {
                for s in eb {
                    if self.compile_stmt(s)? { else_returned = true; break; }
                }
            }
            self.pop_scope();
            if !else_returned {
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }
        } else {
            // else bloğu yok → else_returned = false (merge'e düşüyor)
        }

        self.builder.position_at_end(merge_bb);

        // Her iki dal da return yaptıysa merge block ulaşılamaz
        if then_returned && else_returned {
            self.builder.build_unreachable()
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            return Ok(true);
        }

        Ok(false)
    }

    // ── while döngüsü ────────────────────────────────────────────────────────

    fn compile_while(&mut self, cond: &Expr, body: &[Stmt]) -> CgResult<bool> {
        let cur_fn  = self.cur_fn.unwrap();
        let cond_bb = self.ctx.append_basic_block(cur_fn, "while.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "while.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "while.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cond_bb);
        let cond_val = self.compile_expr(cond)?
            .ok_or_else(|| CodeGenError::new("while condition has no value"))?;
        let cond_bool = self.to_bool(cond_val)?;
        self.builder.build_conditional_branch(cond_bool, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        self.push_scope();
        for s in body {
            if self.compile_stmt(s)? { break; }
        }
        self.pop_scope();
        if !self.current_block_terminated() {
            self.builder.build_unconditional_branch(cond_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.builder.position_at_end(exit_bb);
        Ok(false)
    }

    // ── for döngüsü ──────────────────────────────────────────────────────────

    fn compile_for(
        &mut self,
        init : &Stmt,
        cond : &Expr,
        step : &Expr,
        body : &[Stmt],
    ) -> CgResult<bool> {
        let cur_fn  = self.cur_fn.unwrap();
        self.push_scope();
        self.compile_stmt(init)?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "for.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "for.body");
        let step_bb = self.ctx.append_basic_block(cur_fn, "for.step");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "for.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cond_bb);
        let cv = self.compile_expr(cond)?
            .ok_or_else(|| CodeGenError::new("for condition has no value"))?;
        let cb = self.to_bool(cv)?;
        self.builder.build_conditional_branch(cb, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        for s in body {
            if self.compile_stmt(s)? { break; }
        }
        if !self.current_block_terminated() {
            self.builder.build_unconditional_branch(step_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.builder.position_at_end(step_bb);
        self.compile_expr(step)?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        self.pop_scope();
        Ok(false)
    }

    // ── switch ───────────────────────────────────────────────────────────────

    fn compile_switch(&mut self, expr: &Expr, cases: &[SwitchCase]) -> CgResult<bool> {
        // switch → if/else zinciri
        let val = self.compile_expr(expr)?;
        let cur_fn = self.cur_fn.unwrap();
        let exit_bb = self.ctx.append_basic_block(cur_fn, "switch.exit");

        let mut all_cases_return = true;

        for case in cases {
            let case_val = self.compile_expr(&case.pattern)?;
            if let (Some(v), Some(cv)) = (val.as_ref(), case_val) {
                let eq = self.build_eq(*v, cv)?;
                let then_bb = self.ctx.append_basic_block(cur_fn, "case.body");
                let next_bb = self.ctx.append_basic_block(cur_fn, "case.next");
                self.builder.build_conditional_branch(eq, then_bb, next_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(then_bb);
                self.push_scope();
                let mut ret = false;
                for s in &case.body {
                    if self.compile_stmt(s)? { ret = true; break; }
                }
                self.pop_scope();
                if !ret {
                    all_cases_return = false;
                    self.builder.build_unconditional_branch(exit_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
                self.builder.position_at_end(next_bb);
            }
        }

        // "no case matched" path → exit_bb
        self.builder.build_unconditional_branch(exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);

        // Tüm case'ler return yaptıysa exit_bb ulaşılamaz
        if all_cases_return && !cases.is_empty() {
            self.builder.build_unreachable()
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            return Ok(true);
        }

        Ok(false)
    }

    // ── Expression → LLVM IR ─────────────────────────────────────────────────

    fn compile_expr(&mut self, expr: &Expr) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        match expr {
            // ── Literaller ────────────────────────────────────────────────────
            Expr::IntLit(n) => {
                let v = self.ctx.i64_type().const_int(*n as u64, *n < 0);
                Ok(Some(v.into()))
            }
            Expr::FloatLit(f) => {
                let v = self.ctx.f64_type().const_float(*f);
                Ok(Some(v.into()))
            }
            Expr::BoolLit(b) => {
                let v = self.ctx.bool_type().const_int(*b as u64, false);
                Ok(Some(v.into()))
            }
            Expr::StrLit(s) => {
                let g = self.build_global_string(s)?;
                Ok(Some(g.into()))
            }
            Expr::StrInterp(parts) => {
                self.compile_str_interp(parts)
            }
            Expr::NullLit => {
                let v = self.ctx.ptr_type(AddressSpace::default()).const_null();
                Ok(Some(v.into()))
            }

            // ── Değişken okuma ────────────────────────────────────────────────
            Expr::Ident(name) => {
                if let Some(slot) = self.lookup_var(name).cloned() {
                    let v = self.builder.build_load(slot.ty, slot.ptr, name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(v))
                } else {
                    Ok(None)
                }
            }

            // ── IO.print() ────────────────────────────────────────────────────
            Expr::StaticCall { class, method, args }
                if class == "IO" && method == "print" =>
            {
                self.compile_io_print(args)?;
                Ok(None)
            }

            // ── Stdlib çağrıları ──────────────────────────────────────────────
            Expr::StaticCall { class, method, args } if
                matches!(class.as_str(), "IO"|"Math"|"Time"|"Memory") =>
            {
                self.compile_stdlib_call(class, method, args)
            }

            // ── Diğer static çağrılar ─────────────────────────────────────────
            Expr::StaticCall { class, method, args } => {
                self.compile_static_call(class, method, args)
            }

            // ── Instance metod çağrısı ─────────────────────────────────────────
            Expr::MethodCall { object, method, args } => {
                // Koleksiyon metod kontrolü
                let obj_class = self.infer_object_class(object);
                if let Some(cls) = obj_class.as_deref() {
                    if matches!(cls, "__List" | "__HashMap" | "__Pair") {
                        let cls_owned = cls.to_string();
                        return self.compile_collection_method(object, &cls_owned, method, args);
                    }
                }
                // IO.print gibi statik benzer çağrılar da buraya düşebilir
                if let Expr::Ident(class_name) = object.as_ref() {
                    // Statik metod gibi çağrı (nesne ismi ile)
                    let fn_name = format!("{}_{}", class_name, method);
                    if self.fns.contains_key(&fn_name)
                        || self.module.get_function(&fn_name).is_some()
                    {
                        return self.compile_static_call(class_name, method, args);
                    }
                }
                self.compile_instance_method_call(object, method, args)
            }

            // ── Constructor çağrısı ───────────────────────────────────────────
            Expr::ConstructorCall { class, args } => {
                // Koleksiyon constructor'ları
                match class.as_str() {
                    "List" | "List.of" | "List.empty" => {
                        let f = self.module.get_function("arc_list_new").unwrap();
                        let call = self.builder.build_call(f, &[], "list")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(call.try_as_basic_value().basic());
                    }
                    "HashMap" | "TreeMap" | "HashMap.of" | "HashMap.create"
                    | "TreeMap.create" => {
                        let f = self.module.get_function("arc_map_new").unwrap();
                        let call = self.builder.build_call(f, &[], "map")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(call.try_as_basic_value().basic());
                    }
                    "Pair" | "Pair.of" => {
                        let compiled: Vec<BasicValueEnum<'ctx>> = args.iter()
                            .filter_map(|a| self.compile_expr(a).ok().flatten())
                            .collect();
                        let fst_raw = compiled.get(0).copied();
                        let snd_raw = compiled.get(1).copied();
                        let fst = if let Some(v) = fst_raw {
                            self.value_to_i64(v)?
                        } else {
                            self.ctx.i64_type().const_int(0, false)
                        };
                        let snd = if let Some(v) = snd_raw {
                            self.value_to_i64(v)?
                        } else {
                            self.ctx.i64_type().const_int(0, false)
                        };
                        let f = self.module.get_function("arc_pair_new").unwrap();
                        let call = self.builder.build_call(f, &[fst.into(), snd.into()], "pair")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(call.try_as_basic_value().basic());
                    }
                    _ => {}
                }

                let ctor_name = format!("{}_new", class);
                let compiled: Vec<BasicValueEnum<'ctx>> = args.iter()
                    .filter_map(|a| self.compile_expr(a).ok().flatten())
                    .collect();

                if let Some(ctor_fn) = self.fns.get(&ctor_name).copied()
                    .or_else(|| self.module.get_function(&ctor_name))
                {
                    let meta: Vec<inkwell::values::BasicMetadataValueEnum> =
                        compiled.iter().map(|v| (*v).into()).collect();
                    let call = self.builder.build_call(ctor_fn, &meta, "obj")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(call.try_as_basic_value().basic())
                } else {
                    Ok(None)
                }
            }

            // ── Binary operatörler ────────────────────────────────────────────
            Expr::BinOp { op, left, right } => {
                self.compile_binop(op, left, right)
            }

            // ── Unary operatörler ─────────────────────────────────────────────
            Expr::UnaryOp { op, expr } => {
                self.compile_unary(op, expr)
            }

            // ── Type cast ─────────────────────────────────────────────────────
            Expr::Cast { expr, ty } => {
                let val = self.compile_expr(expr)?;
                if let (Some(v), Some(target_ty)) = (val, self.llvm_type(ty)) {
                    let casted = self.build_cast(v, target_ty)?;
                    Ok(Some(casted))
                } else {
                    Ok(None)
                }
            }

            // ── Ternary ───────────────────────────────────────────────────────
            Expr::Ternary { cond, then, else_ } => {
                let cur_fn   = self.cur_fn.unwrap();
                let cond_val = self.compile_expr(cond)?
                    .ok_or_else(|| CodeGenError::new("ternary condition empty"))?;
                let cond_b   = self.to_bool(cond_val)?;
                let then_bb  = self.ctx.append_basic_block(cur_fn, "tern.then");
                let else_bb  = self.ctx.append_basic_block(cur_fn, "tern.else");
                let merge_bb = self.ctx.append_basic_block(cur_fn, "tern.merge");
                self.builder.build_conditional_branch(cond_b, then_bb, else_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(then_bb);
                let then_val = self.compile_expr(then)?;
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let then_end = self.builder.get_insert_block().unwrap();
                self.builder.position_at_end(else_bb);
                let else_val = self.compile_expr(else_)?;
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let else_end = self.builder.get_insert_block().unwrap();
                self.builder.position_at_end(merge_bb);
                if let (Some(tv), Some(ev)) = (then_val, else_val) {
                    let phi = self.builder.build_phi(tv.get_type(), "tern.phi")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    phi.add_incoming(&[(&tv, then_end), (&ev, else_end)]);
                    Ok(Some(phi.as_basic_value()))
                } else {
                    Ok(None)
                }
            }

            // ── Index ─────────────────────────────────────────────────────────
            Expr::Index { object, index } => {
                let obj = self.compile_expr(object)?;
                let idx = self.compile_expr(index)?;
                if let (Some(o), Some(i)) = (obj, idx) {
                    // Array element: unsafe GEP
                    let i64_idx = self.to_i64(i)?;
                    if let BasicValueEnum::PointerValue(ptr) = o {
                        let elem_ty = self.ctx.i64_type();
                        let gep = unsafe {
                            self.builder.build_gep(elem_ty, ptr, &[i64_idx], "arr.elem")
                                .map_err(|e| CodeGenError::new(e.to_string()))?
                        };
                        let v = self.builder.build_load(BasicTypeEnum::IntType(elem_ty), gep, "arr.load")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(Some(v));
                    }
                }
                Ok(None)
            }

            // ── this ─────────────────────────────────────────────────────────
            Expr::This => {
                if let Some(slot) = self.lookup_var("this").cloned() {
                    let v = self.builder.build_load(slot.ty, slot.ptr, "this_val")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(v))
                } else {
                    Ok(None)
                }
            }
            Expr::Super => Ok(None),

            // ── await ─────────────────────────────────────────────────────────
            Expr::Await(inner) => self.compile_expr(inner),

            // ── Match ─────────────────────────────────────────────────────────
            Expr::Match { expr, arms } => {
                // Şimdilik basit: sadece ilk arm
                let _ = (expr, arms);
                Ok(None)
            }

            // ── Lambda ───────────────────────────────────────────────────────
            Expr::Lambda { .. } => Ok(None),

            // ── FieldAccess ───────────────────────────────────────────────────
            Expr::FieldAccess { object, field } => {
                if let Expr::Ident(class_or_enum) = object.as_ref() {
                    // Enum.Variant → integer sabit
                    if let Some(val) = self.enum_variant_value(class_or_enum, field) {
                        let iv = self.ctx.i32_type().const_int(val as u64, false);
                        return Ok(Some(iv.into()));
                    }
                    // ClassName.staticField → global değişken okuma
                    let global_name = format!("{}_{}", class_or_enum, field);
                    if let Some(gv) = self.module.get_global(&global_name) {
                        let ptr  = gv.as_pointer_value();
                        if let Some(g_ty) = self.any_to_basic(gv.get_value_type()) {
                            let val = self.builder.build_load(g_ty, ptr, field)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            return Ok(Some(val));
                        }
                    }
                }
                self.compile_field_load(object, field)
            }
            Expr::NullSafeAccess { .. } => Ok(None),
        }
    }

    // ── String interpolation → sprintf tabanlı string üretimi ────────────────
    //
    // IO.print içinde StrInterp doğrudan printf ile işlenir.
    // Diğer bağlamlarda (return, VarDecl) sprintf ile gerçek string üretilir.

    fn compile_str_interp(
        &mut self,
        parts: &[StringPart],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        // sprintf'i bildir
        if self.module.get_function("sprintf").is_none() {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i32_ty = self.ctx.i32_type();
            let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], true);
            self.module.add_function("sprintf", ft, None);
        }

        let mut fmt_str   = String::new();
        let mut interp_vals: Vec<BasicValueEnum<'ctx>> = Vec::new();

        for part in parts {
            match part {
                StringPart::Text(t) => {
                    fmt_str.push_str(&t.replace('%', "%%"));
                }
                StringPart::Interp(inner_expr) => {
                    if let Some(val) = self.compile_expr(inner_expr)? {
                        let spec = match val {
                            BasicValueEnum::IntValue(iv) => {
                                match iv.get_type().get_bit_width() {
                                    64 => "%lld",
                                    _  => "%d",
                                }
                            }
                            BasicValueEnum::FloatValue(_) => "%g",
                            BasicValueEnum::PointerValue(_) => "%s",
                            _ => "%d",
                        };
                        fmt_str.push_str(spec);
                        // float promote, i8/i16 → i32
                        let promoted = match val {
                            BasicValueEnum::FloatValue(f) => {
                                let f64ty = self.ctx.f64_type();
                                if f.get_type().get_bit_width() < 64 {
                                    self.builder.build_float_ext(f, f64ty, "fpext")
                                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                                } else { val }
                            }
                            BasicValueEnum::IntValue(iv)
                                if iv.get_type().get_bit_width() < 32 =>
                            {
                                let i32ty = self.ctx.i32_type();
                                self.builder.build_int_s_extend(iv, i32ty, "sext")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                            }
                            _ => val,
                        };
                        interp_vals.push(promoted);
                    } else {
                        fmt_str.push_str("(null)");
                    }
                }
            }
        }

        // malloc(1024) — heap buffer, fonksiyon sonrasında da geçerli
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let i64_ty    = self.ctx.i64_type();
        let sz        = i64_ty.const_int(1024, false);
        let buf_call  = self.builder.build_call(malloc_fn, &[sz.into()], "interp_buf")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let buf_ptr = match buf_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(None),
        };

        // sprintf(buf, fmt, args...)
        let fmt_ptr = self.build_global_string(&fmt_str)?;
        let sprintf_fn = self.module.get_function("sprintf").unwrap();
        let mut sprintf_args: Vec<inkwell::values::BasicMetadataValueEnum> =
            vec![buf_ptr.into(), fmt_ptr.into()];
        for v in interp_vals { sprintf_args.push(v.into()); }
        self.builder.build_call(sprintf_fn, &sprintf_args, "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        Ok(Some(buf_ptr.into()))
    }

    // ── IO.print() ───────────────────────────────────────────────────────────

    // ── Stdlib çağrıları ─────────────────────────────────────────────────────

    fn compile_stdlib_call(
        &mut self,
        class  : &str,
        method : &str,
        args   : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        match (class, method) {
            // IO
            ("IO", "print") => {
                self.compile_io_print(args)?;
                Ok(None)
            }
            ("IO", "read") => {
                // TODO: stdin okuma — şimdilik boş string
                let s = self.build_global_string("")?;
                Ok(Some(s.into()))
            }

            // Math
            ("Math", "sqrt") => {
                self.declare_math_fns();
                if let Some(arg) = args.first() {
                    if let Some(v) = self.compile_expr(arg)? {
                        let f64v = self.to_f64(v)?;
                        let sqrt_fn = self.module.get_function("sqrt").unwrap();
                        let r = self.builder.build_call(sqrt_fn, &[f64v.into()], "sqrt")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(r.try_as_basic_value().basic());
                    }
                }
                Ok(None)
            }
            ("Math", "abs") => {
                if let Some(arg) = args.first() {
                    if let Some(v) = self.compile_expr(arg)? {
                        match v {
                            BasicValueEnum::FloatValue(f) => {
                                self.declare_math_fns();
                                let fabs = self.module.get_function("fabs").unwrap();
                                let r = self.builder.build_call(fabs, &[f.into()], "abs")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                return Ok(r.try_as_basic_value().basic());
                            }
                            BasicValueEnum::IntValue(i) => {
                                // abs = max(x, -x)
                                let neg = self.builder.build_int_neg(i, "neg")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                let cmp = self.builder.build_int_compare(
                                    inkwell::IntPredicate::SGE, i, neg, "cmp"
                                ).map_err(|e| CodeGenError::new(e.to_string()))?;
                                let r = self.builder.build_select(cmp, i, neg, "abs")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                return Ok(Some(r));
                            }
                            _ => {}
                        }
                    }
                }
                Ok(None)
            }
            ("Math", "pow") => {
                self.declare_math_fns();
                let vals: Vec<_> = args.iter()
                    .filter_map(|a| self.compile_expr(a).ok().flatten())
                    .collect();
                if vals.len() >= 2 {
                    let a = self.to_f64(vals[0])?;
                    let b = self.to_f64(vals[1])?;
                    let pow_fn = self.module.get_function("pow").unwrap();
                    let r = self.builder.build_call(pow_fn, &[a.into(), b.into()], "pow")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(r.try_as_basic_value().basic());
                }
                Ok(None)
            }
            ("Math", "PI") | ("Math", "E") => {
                let val = if method == "PI" { std::f64::consts::PI } else { std::f64::consts::E };
                Ok(Some(self.ctx.f64_type().const_float(val).into()))
            }

            // Time — stub implementasyonlar
            ("Time", "now") => {
                // Şimdi için sabit string — gerçek impl Faza 5'te
                let s = self.build_global_string("2026-01-01")?;
                Ok(Some(s.into()))
            }
            ("Time", "generateId") => {
                // Unique ID için static counter kullan
                self.declare_arc_runtime();
                let id_fn = self.module.get_function("arc_generate_id");
                if let Some(f) = id_fn {
                    let r = self.builder.build_call(f, &[], "id")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(r.try_as_basic_value().basic())
                } else {
                    // Fallback: sabit string
                    let s = self.build_global_string("id-1")?;
                    Ok(Some(s.into()))
                }
            }

            // Memory
            ("Memory", "alloc") => {
                if let Some(arg) = args.first() {
                    if let Some(v) = self.compile_expr(arg)? {
                        let malloc = self.module.get_function("malloc")
                            .ok_or_else(|| CodeGenError::new("malloc not declared"))?;
                        let size = self.to_i64(v)?;
                        let r = self.builder.build_call(malloc, &[size.into()], "alloc")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(r.try_as_basic_value().basic());
                    }
                }
                Ok(None)
            }
            ("Memory", "free") => {
                if let Some(arg) = args.first() {
                    if let Some(v) = self.compile_expr(arg)? {
                        let free = self.module.get_function("free")
                            .or_else(|| {
                                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                                let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
                                Some(self.module.add_function("free", ft, None))
                            }).unwrap();
                        self.builder.build_call(free, &[v.into()], "")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                }
                Ok(None)
            }
            ("Memory", "set") | ("Memory", "copy") => {
                // TODO: memset/memcpy
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }

            _ => {
                // Bilinmeyen stdlib çağrısı — argümanları derle, None döndür
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }
        }
    }

    // ── Koleksiyon runtime — saf LLVM IR olarak üretilir ─────────────────────
    //
    // Flat-array düzeni (basit, yeterli):
    //   ArcList  : malloc(8 * 257)  — [0]=len, [1..256]=data
    //   ArcMap   : malloc(8 * 129)  — [0]=len, [1+i*2]=key_ptr, [2+i*2]=val_i64
    //   ArcPair  : malloc(16)       — [0]=fst, [1]=snd  (i64 olarak)
    //
    // Her fonksiyon arc_generate_id gibi builder ile üretilir.
    // Pipeline değişmez: .arm → LLVM IR → .o → .exe

    fn declare_collection_runtime(&mut self) {
        // Önce gerekli extern'leri bildir
        self.declare_malloc();
        self.declare_strcmp();

        // İmzaları önceden kayıt et (diğer fonksiyonlardan çağırabilmek için)
        self.pre_declare_arc_runtime_fns();

        // Fonksiyon gövdelerini üret
        let prev = self.builder.get_insert_block();

        self.gen_arc_list_new();
        self.gen_arc_list_append();
        self.gen_arc_list_length();
        self.gen_arc_list_get();
        self.gen_arc_list_filter();
        self.gen_arc_map_new();
        self.gen_arc_map_set();
        self.gen_arc_map_get_or_default();
        self.gen_arc_pair_new();
        self.gen_arc_pair_first();
        self.gen_arc_pair_second();

        if let Some(bb) = prev { self.builder.position_at_end(bb); }
    }

    fn declare_strcmp(&mut self) {
        if self.module.get_function("strcmp").is_some() { return; }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32_ty = self.ctx.i32_type();
        let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        self.module.add_function("strcmp", ft, None);
    }

    fn pre_declare_arc_runtime_fns(&mut self) {
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty  = self.ctx.i64_type();
        let void_ty = self.ctx.void_type();

        let decls: &[(&str, bool, &[inkwell::types::BasicMetadataTypeEnum], bool)] = &[
            // (name, returns_ptr, params, is_void)
        ];
        let _ = decls;

        macro_rules! decl {
            ($name:expr, ptr, [$($p:expr),*]) => {
                if self.module.get_function($name).is_none() {
                    let ps: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![$($p),*];
                    self.module.add_function($name, ptr_ty.fn_type(&ps, false), None);
                }
            };
            ($name:expr, i64, [$($p:expr),*]) => {
                if self.module.get_function($name).is_none() {
                    let ps: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![$($p),*];
                    self.module.add_function($name, i64_ty.fn_type(&ps, false), None);
                }
            };
            ($name:expr, void, [$($p:expr),*]) => {
                if self.module.get_function($name).is_none() {
                    let ps: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![$($p),*];
                    self.module.add_function($name, void_ty.fn_type(&ps, false), None);
                }
            };
        }

        decl!("arc_list_new",    ptr,  []);
        decl!("arc_list_append", void, [ptr_ty.into(), i64_ty.into()]);
        decl!("arc_list_length", i64,  [ptr_ty.into()]);
        decl!("arc_list_get",    i64,  [ptr_ty.into(), i64_ty.into()]);
        decl!("arc_list_filter", ptr,  [ptr_ty.into(), ptr_ty.into()]);
        decl!("arc_map_new",             ptr,  []);
        decl!("arc_map_set",             void, [ptr_ty.into(), ptr_ty.into(), i64_ty.into()]);
        decl!("arc_map_get_or_default",  i64,  [ptr_ty.into(), ptr_ty.into(), i64_ty.into()]);
        decl!("arc_pair_new",    ptr,  [i64_ty.into(), i64_ty.into()]);
        decl!("arc_pair_first",  i64,  [ptr_ty.into()]);
        decl!("arc_pair_second", i64,  [ptr_ty.into()]);
    }

    // ── arc_list_new() → ptr ─────────────────────────────────────────────────
    // malloc(8 * 257): [0]=len, [1..256]=i64 data

    fn gen_arc_list_new(&mut self) {
        let fn_val = match self.module.get_function("arc_list_new") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let malloc  = self.module.get_function("malloc").unwrap();

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        // malloc(8 * 257) bytes
        let sz = i64_ty.const_int(8 * 257, false);
        let ptr = self.builder.build_call(malloc, &[sz.into()], "list_ptr")
            .unwrap().try_as_basic_value().basic().unwrap();
        // Store len=0 at offset 0
        if let inkwell::values::BasicValueEnum::PointerValue(p) = ptr {
            self.builder.build_store(p, i64_ty.const_int(0, false)).unwrap();
            self.builder.build_return(Some(&p)).unwrap();
        }
    }

    // ── arc_list_append(ptr list, i64 item) → void ───────────────────────────
    // list[0]++ then list[old_len+1] = item

    fn gen_arc_list_append(&mut self) {
        let fn_val = match self.module.get_function("arc_list_append") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let item     = fn_val.get_nth_param(1).unwrap().into_int_value();

        // len = list[0]
        let len = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), list_ptr, "len"
        ).unwrap().into_int_value();

        // list[len+1] = item
        let idx = self.builder.build_int_add(len, i64_ty.const_int(1, false), "idx").unwrap();
        let elem_ptr = unsafe {
            self.builder.build_gep(i64_ty, list_ptr, &[idx], "elem_ptr").unwrap()
        };
        self.builder.build_store(elem_ptr, item).unwrap();

        // list[0] = len+1
        let new_len = self.builder.build_int_add(len, i64_ty.const_int(1, false), "nl").unwrap();
        self.builder.build_store(list_ptr, new_len).unwrap();

        self.builder.build_return(None).unwrap();
    }

    // ── arc_list_length(ptr list) → i64 ─────────────────────────────────────

    fn gen_arc_list_length(&mut self) {
        let fn_val = match self.module.get_function("arc_list_length") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let len = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), list_ptr, "len"
        ).unwrap();
        self.builder.build_return(Some(&len)).unwrap();
    }

    // ── arc_list_get(ptr list, i64 idx) → i64 ───────────────────────────────

    fn gen_arc_list_get(&mut self) {
        let fn_val = match self.module.get_function("arc_list_get") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let idx      = fn_val.get_nth_param(1).unwrap().into_int_value();

        // real_idx = idx + 1
        let real_idx = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "ri").unwrap();
        let elem_ptr = unsafe {
            self.builder.build_gep(i64_ty, list_ptr, &[real_idx], "ep").unwrap()
        };
        let val = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), elem_ptr, "val"
        ).unwrap();
        self.builder.build_return(Some(&val)).unwrap();
    }

    // ── arc_list_filter(ptr list, ptr fn_ptr) → ptr ──────────────────────────

    fn gen_arc_list_filter(&mut self) {
        let fn_val = match self.module.get_function("arc_list_filter") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();

        let entry_bb  = self.ctx.append_basic_block(fn_val, "entry");
        let cond_bb   = self.ctx.append_basic_block(fn_val, "filter.cond");
        let body_bb   = self.ctx.append_basic_block(fn_val, "filter.body");
        let append_bb = self.ctx.append_basic_block(fn_val, "filter.append");
        let next_bb   = self.ctx.append_basic_block(fn_val, "filter.next");
        let exit_bb   = self.ctx.append_basic_block(fn_val, "filter.exit");

        // entry: out = arc_list_new(), len = arc_list_length(list), idx = 0
        self.builder.position_at_end(entry_bb);
        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let fn_ptr   = fn_val.get_nth_param(1).unwrap().into_pointer_value();

        let new_fn   = self.module.get_function("arc_list_new").unwrap();
        let len_fn   = self.module.get_function("arc_list_length").unwrap();
        let get_fn   = self.module.get_function("arc_list_get").unwrap();
        let app_fn   = self.module.get_function("arc_list_append").unwrap();

        let out = self.builder.build_call(new_fn, &[], "out")
            .unwrap().try_as_basic_value().basic().unwrap();
        let len_v = self.builder.build_call(len_fn, &[list_ptr.into()], "len")
            .unwrap().try_as_basic_value().basic().unwrap().into_int_value();

        let idx_slot = self.builder.build_alloca(i64_ty, "idx_slot").unwrap();
        self.builder.build_store(idx_slot, i64_ty.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        // cond: idx < len
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), idx_slot, "idx"
        ).unwrap().into_int_value();
        let cmp = self.builder.build_int_compare(
            inkwell::IntPredicate::SLT, idx, len_v, "cmp"
        ).unwrap();
        self.builder.build_conditional_branch(cmp, body_bb, exit_bb).unwrap();

        // body: item = get(list, idx), res = fn(item)
        self.builder.position_at_end(body_bb);
        let idx2 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), idx_slot, "idx2"
        ).unwrap().into_int_value();
        let item = self.builder.build_call(get_fn, &[list_ptr.into(), idx2.into()], "item")
            .unwrap().try_as_basic_value().basic().unwrap().into_int_value();

        // Indirect call through fn_ptr: fn_ptr(item) → i64
        let fn_ty = i64_ty.fn_type(&[i64_ty.into()], false);
        let res = self.builder.build_indirect_call(fn_ty, fn_ptr, &[item.into()], "res")
            .unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let is_true = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, res, i64_ty.const_int(0, false), "is_true"
        ).unwrap();
        self.builder.build_conditional_branch(is_true, append_bb, next_bb).unwrap();

        // append: arc_list_append(out, item)
        self.builder.position_at_end(append_bb);
        let out_ptr = out.into_pointer_value();
        self.builder.build_call(app_fn, &[out_ptr.into(), item.into()], "").unwrap();
        self.builder.build_unconditional_branch(next_bb).unwrap();

        // next: idx++
        self.builder.position_at_end(next_bb);
        let idx3 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), idx_slot, "idx3"
        ).unwrap().into_int_value();
        let idx4 = self.builder.build_int_add(idx3, i64_ty.const_int(1, false), "idx4").unwrap();
        self.builder.build_store(idx_slot, idx4).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        // exit
        self.builder.position_at_end(exit_bb);
        self.builder.build_return(Some(&out)).unwrap();
    }

    // ── arc_map_new() → ptr ──────────────────────────────────────────────────
    // malloc(8 * 129): [0]=len, [1+i*2]=key_ptr (i64), [2+i*2]=val (i64)

    fn gen_arc_map_new(&mut self) {
        let fn_val = match self.module.get_function("arc_map_new") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let malloc  = self.module.get_function("malloc").unwrap();

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let sz = i64_ty.const_int(8 * 129, false);
        let ptr = self.builder.build_call(malloc, &[sz.into()], "map_ptr")
            .unwrap().try_as_basic_value().basic().unwrap();
        if let inkwell::values::BasicValueEnum::PointerValue(p) = ptr {
            // len = 0
            self.builder.build_store(p, i64_ty.const_int(0, false)).unwrap();
            self.builder.build_return(Some(&p)).unwrap();
        }
    }

    // ── arc_map_set(ptr map, ptr key, i64 val) → void ───────────────────────
    // Lineer arama; eşleşirse güncelle, yoksa ekle

    fn gen_arc_map_set(&mut self) {
        let fn_val = match self.module.get_function("arc_map_set") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty  = self.ctx.i64_type();
        let strcmp  = self.module.get_function("strcmp").unwrap();

        let entry_bb  = self.ctx.append_basic_block(fn_val, "entry");
        let cond_bb   = self.ctx.append_basic_block(fn_val, "set.cond");
        let check_bb  = self.ctx.append_basic_block(fn_val, "set.check");
        let update_bb = self.ctx.append_basic_block(fn_val, "set.update");
        let next_bb   = self.ctx.append_basic_block(fn_val, "set.next");
        let insert_bb = self.ctx.append_basic_block(fn_val, "set.insert");

        self.builder.position_at_end(entry_bb);
        let map_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let key_ptr = fn_val.get_nth_param(1).unwrap().into_pointer_value();
        let val_v   = fn_val.get_nth_param(2).unwrap().into_int_value();

        let len = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), map_ptr, "len"
        ).unwrap().into_int_value();

        let i_slot = self.builder.build_alloca(i64_ty, "i").unwrap();
        self.builder.build_store(i_slot, i64_ty.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        // cond: i < len
        self.builder.position_at_end(cond_bb);
        let i = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i"
        ).unwrap().into_int_value();
        let lt = self.builder.build_int_compare(
            inkwell::IntPredicate::SLT, i, len, "lt"
        ).unwrap();
        self.builder.build_conditional_branch(lt, check_bb, insert_bb).unwrap();

        // check: existing_key = map[1+i*2]; if strcmp==0 → update
        self.builder.position_at_end(check_bb);
        let i2 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i2"
        ).unwrap().into_int_value();
        // key_idx = 1 + i*2
        let two    = i64_ty.const_int(2, false);
        let one    = i64_ty.const_int(1, false);
        let i2t    = self.builder.build_int_mul(i2, two, "i2t").unwrap();
        let ki     = self.builder.build_int_add(i2t, one, "ki").unwrap();
        let kslot  = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[ki], "kslot").unwrap()
        };
        // load stored key as i64, then inttoptr
        let stored_key_i = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), kslot, "ski"
        ).unwrap().into_int_value();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let stored_key_ptr = self.builder.build_int_to_ptr(stored_key_i, ptr_ty, "skp").unwrap();

        let cmp_res = self.builder.build_call(
            strcmp, &[stored_key_ptr.into(), key_ptr.into()], "cmp"
        ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let cmp_zero = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, cmp_res, self.ctx.i32_type().const_int(0, false), "eq"
        ).unwrap();
        self.builder.build_conditional_branch(cmp_zero, update_bb, next_bb).unwrap();

        // update: map[ki+1] = val; return
        self.builder.position_at_end(update_bb);
        let i3 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i3"
        ).unwrap().into_int_value();
        let i3t   = self.builder.build_int_mul(i3, two, "i3t").unwrap();
        let vi    = self.builder.build_int_add(i3t, i64_ty.const_int(2, false), "vi").unwrap();
        let vslot = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[vi], "vs").unwrap()
        };
        self.builder.build_store(vslot, val_v).unwrap();
        self.builder.build_return(None).unwrap();

        // next: i++
        self.builder.position_at_end(next_bb);
        let i4 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i4"
        ).unwrap().into_int_value();
        let i5 = self.builder.build_int_add(i4, one, "i5").unwrap();
        self.builder.build_store(i_slot, i5).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        // insert: map[1+len*2] = key (as i64), map[2+len*2] = val, len++
        self.builder.position_at_end(insert_bb);
        let len2 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), map_ptr, "len2"
        ).unwrap().into_int_value();
        let l2t  = self.builder.build_int_mul(len2, two, "l2t").unwrap();
        let ki2  = self.builder.build_int_add(l2t, one, "ki2").unwrap();
        let vi2  = self.builder.build_int_add(l2t, i64_ty.const_int(2, false), "vi2").unwrap();

        // store key as i64 (ptrtoint)
        let key_as_i = self.builder.build_ptr_to_int(key_ptr, i64_ty, "kasi").unwrap();
        let kslot2 = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[ki2], "ksl2").unwrap()
        };
        self.builder.build_store(kslot2, key_as_i).unwrap();

        let vslot2 = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[vi2], "vsl2").unwrap()
        };
        self.builder.build_store(vslot2, val_v).unwrap();

        let new_len = self.builder.build_int_add(len2, one, "nl").unwrap();
        self.builder.build_store(map_ptr, new_len).unwrap();
        self.builder.build_return(None).unwrap();
    }

    // ── arc_map_get_or_default(ptr map, ptr key, i64 def) → i64 ─────────────

    fn gen_arc_map_get_or_default(&mut self) {
        let fn_val = match self.module.get_function("arc_map_get_or_default") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let strcmp = self.module.get_function("strcmp").unwrap();

        let entry_bb  = self.ctx.append_basic_block(fn_val, "entry");
        let cond_bb   = self.ctx.append_basic_block(fn_val, "get.cond");
        let check_bb  = self.ctx.append_basic_block(fn_val, "get.check");
        let found_bb  = self.ctx.append_basic_block(fn_val, "get.found");
        let next_bb   = self.ctx.append_basic_block(fn_val, "get.next");
        let miss_bb   = self.ctx.append_basic_block(fn_val, "get.miss");

        self.builder.position_at_end(entry_bb);
        let map_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let key_ptr = fn_val.get_nth_param(1).unwrap().into_pointer_value();
        let def_v   = fn_val.get_nth_param(2).unwrap().into_int_value();

        let len = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), map_ptr, "len"
        ).unwrap().into_int_value();

        let i_slot = self.builder.build_alloca(i64_ty, "i").unwrap();
        self.builder.build_store(i_slot, i64_ty.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        // cond
        self.builder.position_at_end(cond_bb);
        let i = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i"
        ).unwrap().into_int_value();
        let lt = self.builder.build_int_compare(
            inkwell::IntPredicate::SLT, i, len, "lt"
        ).unwrap();
        self.builder.build_conditional_branch(lt, check_bb, miss_bb).unwrap();

        // check
        self.builder.position_at_end(check_bb);
        let ic = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "ic"
        ).unwrap().into_int_value();
        let two = i64_ty.const_int(2, false);
        let one = i64_ty.const_int(1, false);
        let ict  = self.builder.build_int_mul(ic, two, "ict").unwrap();
        let ki   = self.builder.build_int_add(ict, one, "ki").unwrap();
        let kslot = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[ki], "ks").unwrap()
        };
        let ski = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), kslot, "ski"
        ).unwrap().into_int_value();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let skp = self.builder.build_int_to_ptr(ski, ptr_ty, "skp").unwrap();
        let cmp = self.builder.build_call(
            strcmp, &[skp.into(), key_ptr.into()], "cmp"
        ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let eq = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "eq"
        ).unwrap();
        self.builder.build_conditional_branch(eq, found_bb, next_bb).unwrap();

        // found: return map[ki+1]
        self.builder.position_at_end(found_bb);
        let if2 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "if2"
        ).unwrap().into_int_value();
        let if2t = self.builder.build_int_mul(if2, two, "if2t").unwrap();
        let vi   = self.builder.build_int_add(if2t, i64_ty.const_int(2, false), "vi").unwrap();
        let vslot = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[vi], "vs").unwrap()
        };
        let val = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), vslot, "val"
        ).unwrap();
        self.builder.build_return(Some(&val)).unwrap();

        // next: i++
        self.builder.position_at_end(next_bb);
        let in_ = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "in"
        ).unwrap().into_int_value();
        let in2 = self.builder.build_int_add(in_, one, "in2").unwrap();
        self.builder.build_store(i_slot, in2).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        // miss
        self.builder.position_at_end(miss_bb);
        self.builder.build_return(Some(&def_v)).unwrap();
    }

    // ── arc_pair_new(i64 fst, i64 snd) → ptr ────────────────────────────────
    // malloc(16): [0]=fst, [1]=snd

    fn gen_arc_pair_new(&mut self) {
        let fn_val = match self.module.get_function("arc_pair_new") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let malloc  = self.module.get_function("malloc").unwrap();

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let fst = fn_val.get_nth_param(0).unwrap().into_int_value();
        let snd = fn_val.get_nth_param(1).unwrap().into_int_value();

        let sz = i64_ty.const_int(16, false);
        let p  = self.builder.build_call(malloc, &[sz.into()], "pair_ptr")
            .unwrap().try_as_basic_value().basic().unwrap();

        if let inkwell::values::BasicValueEnum::PointerValue(ptr) = p {
            // [0] = fst
            self.builder.build_store(ptr, fst).unwrap();
            // [1] = snd
            let snd_ptr = unsafe {
                self.builder.build_gep(i64_ty, ptr, &[i64_ty.const_int(1, false)], "snd_ptr").unwrap()
            };
            self.builder.build_store(snd_ptr, snd).unwrap();
            self.builder.build_return(Some(&ptr)).unwrap();
        }
    }

    // ── arc_pair_first(ptr pair) → i64 ──────────────────────────────────────

    fn gen_arc_pair_first(&mut self) {
        let fn_val = match self.module.get_function("arc_pair_first") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let pair_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let val = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), pair_ptr, "fst"
        ).unwrap();
        self.builder.build_return(Some(&val)).unwrap();
    }

    // ── arc_pair_second(ptr pair) → i64 ─────────────────────────────────────

    fn gen_arc_pair_second(&mut self) {
        let fn_val = match self.module.get_function("arc_pair_second") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let pair_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let snd_ptr = unsafe {
            self.builder.build_gep(i64_ty, pair_ptr, &[i64_ty.const_int(1, false)], "sp").unwrap()
        };
        let val = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), snd_ptr, "snd"
        ).unwrap();
        self.builder.build_return(Some(&val)).unwrap();
    }

    fn declare_math_fns(&mut self) {
        let f64_ty = self.ctx.f64_type();
        for (name, nargs) in &[("sqrt", 1usize), ("fabs", 1), ("pow", 2)] {
            if self.module.get_function(name).is_some() { continue; }
            let params: Vec<inkwell::types::BasicMetadataTypeEnum> =
                (0..*nargs).map(|_| f64_ty.into()).collect();
            let ft = f64_ty.fn_type(&params, false);
            self.module.add_function(name, ft, None);
        }
    }

    fn declare_arc_runtime(&mut self) {
        if self.module.get_function("arc_generate_id").is_some() { return; }

        // sprintf declare et
        if self.module.get_function("sprintf").is_none() {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i32_ty = self.ctx.i32_type();
            let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], true);
            self.module.add_function("sprintf", ft, None);
        }

        // arc_generate_id: her çağrıda benzersiz "id-N" string döndür
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();

        // Global counter
        let counter = self.module.add_global(i64_ty, None, "arc_id_counter");
        counter.set_initializer(&i64_ty.const_int(0, false));
        counter.set_linkage(inkwell::module::Linkage::Internal);

        // 32 byte buffer
        let buf_ty = self.ctx.i8_type().array_type(32);
        let buf = self.module.add_global(buf_ty, None, "arc_id_buf");
        buf.set_initializer(&buf_ty.const_zero());
        buf.set_linkage(inkwell::module::Linkage::Internal);

        // format string "id-%lld"
        let fmt_bytes = b"id-%lld\0";
        let fmt_arr = self.ctx.const_string(fmt_bytes, false);
        let fmt_global = self.module.add_global(fmt_arr.get_type(), None, "arc_id_fmt");
        fmt_global.set_initializer(&fmt_arr);
        fmt_global.set_linkage(inkwell::module::Linkage::Internal);

        // Fonksiyon gövdesi
        let ft = ptr_ty.fn_type(&[], false);
        let fn_val = self.module.add_function("arc_generate_id", ft, None);
        let entry = self.ctx.append_basic_block(fn_val, "entry");

        // Builder'ın mevcut konumunu kaydet
        let prev_block = self.builder.get_insert_block();
        self.builder.position_at_end(entry);

        // Counter yükle ve artır
        let n = self.builder.build_load(BasicTypeEnum::IntType(i64_ty), counter.as_pointer_value(), "n")
            .unwrap().into_int_value();
        let n1 = self.builder.build_int_add(n, i64_ty.const_int(1, false), "n1").unwrap();
        self.builder.build_store(counter.as_pointer_value(), n1).unwrap();

        // buf pointer
        let buf_ptr = buf.as_pointer_value();

        // sprintf(buf, fmt, n)
        let sprintf = self.module.get_function("sprintf").unwrap();
        let fmt_ptr = fmt_global.as_pointer_value();
        self.builder.build_call(sprintf, &[buf_ptr.into(), fmt_ptr.into(), n.into()], "").unwrap();

        // buf döndür
        self.builder.build_return(Some(&buf_ptr)).unwrap();

        // Builder'ı eski konuma geri döndür
        if let Some(bb) = prev_block {
            self.builder.position_at_end(bb);
        }
    }

    fn to_f64(&mut self, val: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::FloatValue<'ctx>>
    {
        let f64ty = self.ctx.f64_type();
        match val {
            BasicValueEnum::FloatValue(f) => {
                if f.get_type().get_bit_width() < 64 {
                    self.builder.build_float_ext(f, f64ty, "fpext64")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                } else {
                    Ok(f)
                }
            }
            BasicValueEnum::IntValue(i) => {
                self.builder.build_signed_int_to_float(i, f64ty, "itof")
                    .map_err(|e| CodeGenError::new(e.to_string()))
            }
            _ => Err(CodeGenError::new("cannot convert to f64")),
        }
    }

    fn compile_io_print(&mut self, args: &[Expr]) -> CgResult<()> {
        let printf = self.module.get_function("printf")
            .ok_or_else(|| CodeGenError::new("printf not declared"))?;

        if args.is_empty() {
            let fmt = self.build_global_string("\n")?;
            self.builder.build_call(printf, &[fmt.into()], "print")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            return Ok(());
        }

        match &args[0] {
            // Düz string literal — newline ekle
            Expr::StrLit(s) => {
                let fmt = self.build_global_string(&format!("{}\n", s))?;
                self.builder.build_call(printf, &[fmt.into()], "print")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }

            // String interpolation — ${expr} → printf format specifier
            Expr::StrInterp(parts) => {
                // 1. Format string ve değerleri topla
                let mut fmt_str   = String::new();
                let mut interp_vals: Vec<BasicValueEnum<'ctx>> = Vec::new();

                for part in parts {
                    match part {
                        StringPart::Text(t) => {
                            // % işaretlerini escape et
                            fmt_str.push_str(&t.replace('%', "%%"));
                        }
                        StringPart::Interp(inner_expr) => {
                            if let Some(val) = self.compile_expr(inner_expr)? {
                                // Tipi bakarak format specifier seç
                                let spec = match val {
                                    BasicValueEnum::IntValue(iv) => {
                                        match iv.get_type().get_bit_width() {
                                            64 => "%lld",
                                            _  => "%d",
                                        }
                                    }
                                    BasicValueEnum::FloatValue(_) => "%g",
                                    BasicValueEnum::PointerValue(_) => "%s",
                                    _ => "%d",
                                };
                                fmt_str.push_str(spec);
                                // Float için double promote
                                let promoted = match val {
                                    BasicValueEnum::FloatValue(f) => {
                                        let f64ty = self.ctx.f64_type();
                                        if f.get_type().get_bit_width() < 64 {
                                            self.builder.build_float_ext(f, f64ty, "fpext")
                                                .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                                        } else {
                                            val
                                        }
                                    }
                                    // i8/i16/i32 → i32 promote for printf
                                    BasicValueEnum::IntValue(iv) if iv.get_type().get_bit_width() < 32 => {
                                        let i32ty = self.ctx.i32_type();
                                        self.builder.build_int_s_extend(iv, i32ty, "sext")
                                            .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                                    }
                                    _ => val,
                                };
                                interp_vals.push(promoted);
                            } else {
                                fmt_str.push_str("(null)");
                            }
                        }
                    }
                }
                fmt_str.push('\n');

                let fmt_ptr = self.build_global_string(&fmt_str)?;
                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![fmt_ptr.into()];
                for v in interp_vals {
                    call_args.push(v.into());
                }
                self.builder.build_call(printf, &call_args, "print")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }

            // Diğer expr — değeri doğrudan yazdır
            other => {
                if let Some(val) = self.compile_expr(other)? {
                    let (fmt_s, promoted) = match val {
                        BasicValueEnum::IntValue(iv) => {
                            let spec = if iv.get_type().get_bit_width() == 64 { "%lld\n" } else { "%d\n" };
                            (spec, val)
                        }
                        BasicValueEnum::FloatValue(f) => {
                            let f64ty = self.ctx.f64_type();
                            let prom: BasicValueEnum = if f.get_type().get_bit_width() < 64 {
                                self.builder.build_float_ext(f, f64ty, "fpext")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                            } else { val };
                            ("%g\n", prom)
                        }
                        BasicValueEnum::PointerValue(_) => ("%s\n", val),
                        _ => ("%d\n", val),
                    };
                    let fmt_ptr = self.build_global_string(fmt_s)?;
                    self.builder.build_call(printf, &[fmt_ptr.into(), promoted.into()], "print")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
            }
        }
        Ok(())
    }

    // ── Static çağrı ─────────────────────────────────────────────────────────

    fn compile_static_call(
        &mut self,
        class  : &str,
        method : &str,
        args   : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let fn_name = format!("{}_{}", class, method);

        // Önce gerçek static metod olarak dene
        if let Some(fn_val) = self.fns.get(&fn_name).copied()
            .or_else(|| self.module.get_function(&fn_name))
        {
            let compiled_args: Vec<BasicValueEnum<'ctx>> = args.iter()
                .filter_map(|a| self.compile_expr(a).ok().flatten())
                .collect();
            let meta_args: Vec<inkwell::values::BasicMetadataValueEnum> =
                compiled_args.iter().map(|v| (*v).into()).collect();
            let call = self.builder.build_call(fn_val, &meta_args, "call")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            return Ok(call.try_as_basic_value().basic());
        }

        // Bulunamadı — belki 'class' bir değişken ismi (instance method call)
        // Parser StaticCall üretir: c.increment() → StaticCall("c","increment")
        let obj_expr = Expr::Ident(class.to_string());

        // Koleksiyon değişkeni kontrolü (List, HashMap, Pair)
        if let Some(col_cls) = self.infer_object_class(&obj_expr) {
            if matches!(col_cls.as_str(), "__List" | "__HashMap" | "__Pair") {
                let col_cls_owned = col_cls.clone();
                return self.compile_collection_method(&obj_expr.clone(), &col_cls_owned, method, args);
            }
        }

        if let Some(class_name) = self.infer_object_class(&obj_expr) {
            let instance_fn = format!("{}_{}", class_name, method);
            if let Some(fn_val) = self.fns.get(&instance_fn).copied()
                .or_else(|| self.module.get_function(&instance_fn))
            {
                // this pointer'ı yükle
                let this_ptr = if let Some(slot) = self.lookup_var(class).cloned() {
                    self.builder.build_load(slot.ty, slot.ptr, "this_load")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                } else {
                    return Ok(None);
                };

                // Argümanları derle
                let compiled_args: Vec<BasicValueEnum<'ctx>> = args.iter()
                    .filter_map(|a| self.compile_expr(a).ok().flatten())
                    .collect();

                let mut meta_args: Vec<inkwell::values::BasicMetadataValueEnum> =
                    vec![this_ptr.into()];
                for v in &compiled_args { meta_args.push((*v).into()); }

                let call = self.builder.build_call(fn_val, &meta_args, "mcall")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                return Ok(call.try_as_basic_value().basic());
            }
        }

        Ok(None)
    }

    // ── Instance metod çağrısı ────────────────────────────────────────────────

    fn compile_method_call(
        &mut self,
        _object : &Expr,
        _method : &str,
        args    : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        for a in args { self.compile_expr(a)?; }
        Ok(None)
    }

    // ── Instance metod çağrısı ────────────────────────────────────────────────

    fn compile_instance_method_call(
        &mut self,
        object : &Expr,
        method : &str,
        args   : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        // Nesnenin tipi — class adını bul
        let class_name = self.infer_object_class(object);

        // this pointer'ı derle
        let this_ptr = match self.compile_expr(object)? {
            Some(v) => v,
            None    => return Ok(None),
        };

        // Metod fonksiyonunu bul
        let fn_name = match &class_name {
            Some(c) => format!("{}_{}", c, method),
            None    => return Ok(None),
        };

        let fn_val = match self.fns.get(&fn_name).copied()
            .or_else(|| self.module.get_function(&fn_name))
        {
            Some(f) => f,
            None    => return Ok(None),
        };

        // Argümanları derle: this + args
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![this_ptr.into()];
        for a in args {
            if let Some(v) = self.compile_expr(a)? {
                call_args.push(v.into());
            }
        }

        let call = self.builder.build_call(fn_val, &call_args, "mcall")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(call.try_as_basic_value().basic())
    }

    // ── Field okuma ──────────────────────────────────────────────────────────

    fn compile_field_load(
        &mut self,
        object : &Expr,
        field  : &str,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let class_name = self.infer_object_class(object);
        let this_ptr = match self.compile_expr(object)? {
            Some(v) => v,
            None    => return Ok(None),
        };

        if let (Some(cn), BasicValueEnum::PointerValue(ptr)) = (class_name, this_ptr) {
            return self.gep_field_load(&cn, ptr, field);
        }
        Ok(None)
    }

    fn gep_field_load(
        &mut self,
        class : &str,
        ptr   : inkwell::values::PointerValue<'ctx>,
        field : &str,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let idx = match self.field_indices.get(class).and_then(|m| m.get(field)).copied() {
            Some(i) => i,
            None    => return Ok(None),
        };
        let struct_ty = match self.struct_types.get(class).copied() {
            Some(t) => t,
            None    => return Ok(None),
        };
        let i32_ty = self.ctx.i32_type();
        let gep = self.builder.build_struct_gep(struct_ty, ptr, idx, field)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        // Field tipini struct'tan al
        let field_ty = struct_ty.get_field_type_at_index(idx)
            .ok_or_else(|| CodeGenError::new(format!("field {} not found", field)))?;
        let val = self.builder.build_load(field_ty, gep, field)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let _ = i32_ty;
        Ok(Some(val))
    }

    fn gep_field_store(
        &mut self,
        class : &str,
        ptr   : inkwell::values::PointerValue<'ctx>,
        field : &str,
        val   : BasicValueEnum<'ctx>,
    ) -> CgResult<()> {
        let idx = match self.field_indices.get(class).and_then(|m| m.get(field)).copied() {
            Some(i) => i,
            None    => return Ok(()),
        };
        let struct_ty = match self.struct_types.get(class).copied() {
            Some(t) => t,
            None    => return Ok(()),
        };
        let gep = self.builder.build_struct_gep(struct_ty, ptr, idx, &format!("{}.ptr", field))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let field_ty = struct_ty.get_field_type_at_index(idx)
            .ok_or_else(|| CodeGenError::new(format!("field {} not found", field)))?;
        let coerced = self.coerce_value(val, field_ty)?;
        self.builder.build_store(gep, coerced)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(())
    }

    // Nesnenin class adını VarSlot'tan, field tipinden veya cur_class'tan çıkar
    fn infer_object_class(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::This => self.cur_class.clone(),
            Expr::Ident(name) => {
                for scope in self.scopes.iter().rev() {
                    if let Some(slot) = scope.get(name.as_str()) {
                        if slot.class_name.is_some() {
                            return slot.class_name.clone();
                        }
                    }
                }
                None
            }
            // this.field → field'ın tipi
            Expr::FieldAccess { object, field } => {
                let owner_class = self.infer_object_class(object)?;
                // field_arimo_types'ta bu class'ın field tipini bul
                self.field_arimo_types
                    .get(&owner_class)?
                    .get(field.as_str())
                    .cloned()
            }
            _ => None,
        }
    }

    // ── Binary operatörler ────────────────────────────────────────────────────

    fn compile_binop(
        &mut self,
        op    : &BinOp,
        left  : &Expr,
        right : &Expr,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        // Atama özel işlenir
        if matches!(op, BinOp::Assign) {
            return self.compile_assign(left, right);
        }
        // Compound assignment
        if matches!(op, BinOp::AddAssign | BinOp::SubAssign | BinOp::MulAssign | BinOp::DivAssign) {
            return self.compile_compound_assign(op, left, right);
        }

        let lv = self.compile_expr(left)?;
        let rv = self.compile_expr(right)?;

        let (lv, rv) = match (lv, rv) {
            (Some(l), Some(r)) => (l, r),
            _ => return Ok(None),
        };

        let result: BasicValueEnum<'ctx> = match op {
            BinOp::Add => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_add(l, r, "add")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                (BasicValueEnum::FloatValue(l), BasicValueEnum::FloatValue(r)) => {
                    self.builder.build_float_add(l, r, "fadd")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Sub => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_sub(l, r, "sub")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                (BasicValueEnum::FloatValue(l), BasicValueEnum::FloatValue(r)) => {
                    self.builder.build_float_sub(l, r, "fsub")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Mul => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_mul(l, r, "mul")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                (BasicValueEnum::FloatValue(l), BasicValueEnum::FloatValue(r)) => {
                    self.builder.build_float_mul(l, r, "fmul")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Div => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_signed_div(l, r, "div")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                (BasicValueEnum::FloatValue(l), BasicValueEnum::FloatValue(r)) => {
                    self.builder.build_float_div(l, r, "fdiv")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Mod => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_signed_rem(l, r, "rem")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Eq  => { let r = self.build_eq(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Ne  => { let r = self.build_ne(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Lt  => { let r = self.build_lt(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Le  => { let r = self.build_le(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Gt  => { let r = self.build_gt(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Ge  => { let r = self.build_ge(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::And => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_and(l, r, "and")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Or => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_or(l, r, "or")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::BitAnd => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_and(l, r, "band")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::BitOr => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_or(l, r, "bor")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::BitXor => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_xor(l, r, "xor")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Shl => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    // Shift miktarı sol operandla aynı tipte olmalı
                    let r_cast = self.match_int_width(r, l.get_type())?;
                    self.builder.build_left_shift(l, r_cast, "shl")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Shr => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    let r_cast = self.match_int_width(r, l.get_type())?;
                    self.builder.build_right_shift(l, r_cast, false, "shr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        Ok(Some(result))
    }

    fn compile_assign(&mut self, target: &Expr, value: &Expr) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let val = self.compile_expr(value)?;
        if let Some(v) = val {
            match target {
                Expr::Ident(name) => {
                    if let Some(slot) = self.lookup_var(name).cloned() {
                        let coerced = self.coerce_value(v, slot.ty)?;
                        self.builder.build_store(slot.ptr, coerced)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                }
                // this.field = value  veya  ClassName.staticField = value
                Expr::FieldAccess { object, field } => {
                    // Static field yazmayı önce dene
                    if let Expr::Ident(class_name) = object.as_ref() {
                        let global_name = format!("{}_{}", class_name, field);
                        if let Some(gv) = self.module.get_global(&global_name) {
                            let ptr = gv.as_pointer_value();
                            if let Some(g_ty) = self.any_to_basic(gv.get_value_type()) {
                                let coerced = self.coerce_value(v, g_ty)?;
                                self.builder.build_store(ptr, coerced)
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                            }
                        } else {
                            // Instance field yazma
                            let cn = self.infer_object_class(object);
                            let obj_ptr = self.compile_expr(object)?;
                            if let (Some(cn), Some(BasicValueEnum::PointerValue(ptr))) = (cn, obj_ptr) {
                                self.gep_field_store(&cn.clone(), ptr, field, v)?;
                            }
                        }
                    } else {
                        let class_name = self.infer_object_class(object);
                        let obj_ptr = self.compile_expr(object)?;
                        if let (Some(cn), Some(BasicValueEnum::PointerValue(ptr))) = (class_name, obj_ptr) {
                            self.gep_field_store(&cn.clone(), ptr, field, v)?;
                        }
                    }
                }
                _ => {}
            }
            Ok(Some(v))
        } else {
            Ok(None)
        }
    }

    fn compile_compound_assign(
        &mut self,
        op     : &BinOp,
        target : &Expr,
        value  : &Expr,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let bin_op = match op {
            BinOp::AddAssign => BinOp::Add,
            BinOp::SubAssign => BinOp::Sub,
            BinOp::MulAssign => BinOp::Mul,
            BinOp::DivAssign => BinOp::Div,
            _                => return Ok(None),
        };
        let computed = Expr::BinOp {
            op    : bin_op,
            left  : Box::new(target.clone()),
            right : Box::new(value.clone()),
        };
        self.compile_assign(target, &computed)
    }

    // ── Unary operatörler ─────────────────────────────────────────────────────

    fn compile_unary(&mut self, op: &UnaryOp, expr: &Expr) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let val = self.compile_expr(expr)?;
        let val = match val { Some(v) => v, None => return Ok(None) };
        match op {
            UnaryOp::Neg => match val {
                BasicValueEnum::IntValue(v) => {
                    let r = self.builder.build_int_neg(v, "neg")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(r.into()))
                }
                BasicValueEnum::FloatValue(v) => {
                    let r = self.builder.build_float_neg(v, "fneg")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(r.into()))
                }
                _ => Ok(None),
            },
            UnaryOp::Not => match val {
                BasicValueEnum::IntValue(v) => {
                    let r = self.builder.build_not(v, "not")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(r.into()))
                }
                _ => Ok(None),
            },
            UnaryOp::BitNot => match val {
                BasicValueEnum::IntValue(v) => {
                    let r = self.builder.build_not(v, "bitnot")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(r.into()))
                }
                _ => Ok(None),
            },
            UnaryOp::PreInc | UnaryOp::PostInc => {
                let one = self.ctx.i64_type().const_int(1, false);
                self.compile_assign(expr, &Expr::BinOp {
                    op    : BinOp::Add,
                    left  : Box::new(expr.clone()),
                    right : Box::new(Expr::IntLit(1)),
                })?;
                if matches!(op, UnaryOp::PostInc) { Ok(Some(val)) }
                else {
                    let _ = one;
                    self.compile_expr(expr)
                }
            }
            UnaryOp::PreDec | UnaryOp::PostDec => {
                self.compile_assign(expr, &Expr::BinOp {
                    op    : BinOp::Sub,
                    left  : Box::new(expr.clone()),
                    right : Box::new(Expr::IntLit(1)),
                })?;
                if matches!(op, UnaryOp::PostDec) { Ok(Some(val)) }
                else { self.compile_expr(expr) }
            }
        }
    }

    // ── Yardımcılar ──────────────────────────────────────────────────────────

    fn build_global_string(&mut self, s: &str) -> CgResult<PointerValue<'ctx>> {
        let gs = self.builder.build_global_string_ptr(s, "str")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(gs.as_pointer_value())
    }

    /// Shift için: sağ operandı sol operandın genişliğine getir
    fn any_to_basic(&self, any: inkwell::types::AnyTypeEnum<'ctx>) -> Option<BasicTypeEnum<'ctx>> {
        use inkwell::types::AnyTypeEnum;
        match any {
            AnyTypeEnum::IntType(t)     => Some(t.into()),
            AnyTypeEnum::FloatType(t)   => Some(t.into()),
            AnyTypeEnum::PointerType(t) => Some(t.into()),
            AnyTypeEnum::ArrayType(t)   => Some(t.into()),
            AnyTypeEnum::StructType(t)  => Some(t.into()),
            AnyTypeEnum::VectorType(t)  => Some(t.into()),
            _ => None,
        }
    }

    fn match_int_width(
        &mut self,
        val  : inkwell::values::IntValue<'ctx>,
        dest : inkwell::types::IntType<'ctx>,
    ) -> CgResult<inkwell::values::IntValue<'ctx>> {
        let src_bits = val.get_type().get_bit_width();
        let dst_bits = dest.get_bit_width();
        if src_bits == dst_bits { return Ok(val); }
        if src_bits < dst_bits {
            self.builder.build_int_z_extend(val, dest, "shamt_zext")
                .map_err(|e| CodeGenError::new(e.to_string()))
        } else {
            self.builder.build_int_truncate(val, dest, "shamt_trunc")
                .map_err(|e| CodeGenError::new(e.to_string()))
        }
    }

    fn to_bool(&mut self, val: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match val {
            BasicValueEnum::IntValue(v) => {
                let bty = self.ctx.bool_type();
                if v.get_type().get_bit_width() == 1 {
                    Ok(v)
                } else {
                    let zero = v.get_type().const_int(0, false);
                    self.builder.build_int_compare(
                        inkwell::IntPredicate::NE, v, zero, "tobool"
                    ).map_err(|e| CodeGenError::new(e.to_string()))
                }
            }
            BasicValueEnum::PointerValue(p) => {
                let null = p.get_type().const_null();
                self.builder.build_int_compare(
                    inkwell::IntPredicate::NE, p, null, "ptrbool"
                ).map_err(|e| CodeGenError::new(e.to_string()))
            }
            _ => Err(CodeGenError::new("cannot convert to bool")),
        }
    }

    fn to_i64(&mut self, val: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match val {
            BasicValueEnum::IntValue(v) => {
                let i64ty = self.ctx.i64_type();
                if v.get_type().get_bit_width() == 64 {
                    Ok(v)
                } else {
                    self.builder.build_int_z_extend(v, i64ty, "zext")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                }
            }
            _ => Err(CodeGenError::new("cannot convert to i64")),
        }
    }

    fn build_eq(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::EQ, a, b, "eq")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            (BasicValueEnum::FloatValue(a), BasicValueEnum::FloatValue(b)) =>
                self.builder.build_float_compare(inkwell::FloatPredicate::OEQ, a, b, "feq")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn build_ne(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::NE, a, b, "ne")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(1, false)),
        }
    }

    fn build_lt(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::SLT, a, b, "lt")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            (BasicValueEnum::FloatValue(a), BasicValueEnum::FloatValue(b)) =>
                self.builder.build_float_compare(inkwell::FloatPredicate::OLT, a, b, "flt")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn build_le(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::SLE, a, b, "le")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn build_gt(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::SGT, a, b, "gt")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn build_ge(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::SGE, a, b, "ge")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn coerce_value(
        &mut self,
        val     : BasicValueEnum<'ctx>,
        target  : BasicTypeEnum<'ctx>,
    ) -> CgResult<BasicValueEnum<'ctx>> {
        if val.get_type() == target { return Ok(val); }
        match (val, target) {
            (BasicValueEnum::IntValue(v), BasicTypeEnum::IntType(t)) => {
                let bits_src = v.get_type().get_bit_width();
                let bits_dst = t.get_bit_width();
                if bits_src < bits_dst {
                    Ok(self.builder.build_int_z_extend(v, t, "zext")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into())
                } else if bits_src > bits_dst {
                    Ok(self.builder.build_int_truncate(v, t, "trunc")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into())
                } else {
                    Ok(val)
                }
            }
            (BasicValueEnum::IntValue(v), BasicTypeEnum::FloatType(t)) => {
                Ok(self.builder.build_signed_int_to_float(v, t, "itof")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into())
            }
            _ => Ok(val),
        }
    }

    fn build_cast(
        &mut self,
        val     : BasicValueEnum<'ctx>,
        target  : BasicTypeEnum<'ctx>,
    ) -> CgResult<BasicValueEnum<'ctx>> {
        self.coerce_value(val, target)
    }

    fn current_block_terminated(&self) -> bool {
        if let Some(bb) = self.builder.get_insert_block() {
            bb.get_terminator().is_some()
        } else {
            false
        }
    }

    // ── Koleksiyon metod dispatch ─────────────────────────────────────────────

    fn compile_collection_method(
        &mut self,
        object     : &Expr,
        collection : &str,
        method     : &str,
        args       : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();

        // Nesneyi derle → pointer değeri al
        let obj_val = match self.compile_expr(object)? {
            Some(v) => v,
            None    => return Ok(None),
        };
        let list_ptr = match obj_val {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Ok(None),
        };

        match (collection, method) {
            // ── List metodları ───────────────────────────────────────────────
            ("__List", "append") => {
                let item_val = if let Some(a) = args.first() {
                    self.compile_expr(a)?.map(|v| self.value_to_i64(v)).transpose()?
                        .unwrap_or_else(|| i64_ty.const_int(0, false))
                } else {
                    i64_ty.const_int(0, false)
                };
                let f = self.module.get_function("arc_list_append").unwrap();
                self.builder.build_call(f, &[list_ptr.into(), item_val.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(None)
            }

            ("__List", "length") => {
                let f = self.module.get_function("arc_list_length").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into()], "list_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            ("__List", "isEmpty") => {
                let f = self.module.get_function("arc_list_length").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into()], "list_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(len)) = r.try_as_basic_value().basic() {
                    let zero = i64_ty.const_int(0, false);
                    let eq = self.builder.build_int_compare(
                        inkwell::IntPredicate::EQ, len, zero, "is_empty"
                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(eq.into()));
                }
                Ok(None)
            }

            ("__List", "filter") => {
                // Elem class'ı bul (lambda parametre tipi için)
                let elem_cls = self.infer_elem_class(object);
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    let fn_ptr = self.compile_lambda_for_filter(lambda, elem_cls)?;
                    if let Some(fn_ptr_val) = fn_ptr {
                        let f = self.module.get_function("arc_list_filter").unwrap();
                        let r = self.builder.build_call(
                            f,
                            &[list_ptr.into(), fn_ptr_val.into()],
                            "filtered"
                        ).map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(r.try_as_basic_value().basic());
                    }
                }
                // Lambda derlenemezse boş liste döndür
                let f = self.module.get_function("arc_list_new").unwrap();
                let r = self.builder.build_call(f, &[], "empty_list")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            // take / takeLast / sortedBy — şimdilik listeyi olduğu gibi döndür
            ("__List", "take") | ("__List", "takeLast") | ("__List", "sortedBy") => {
                for a in args { self.compile_expr(a)?; }
                Ok(Some(list_ptr.into()))
            }

            ("__List", "reduce") => {
                for a in args { self.compile_expr(a)?; }
                Ok(Some(i64_ty.const_int(0, false).into()))
            }

            // ── HashMap metodları ────────────────────────────────────────────
            ("__HashMap", "set") => {
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };

                let val = if let Some(a) = args.get(1) {
                    self.compile_expr(a)?.map(|v| self.value_to_i64(v)).transpose()?
                        .unwrap_or_else(|| i64_ty.const_int(0, false))
                } else { i64_ty.const_int(0, false) };

                let f = self.module.get_function("arc_map_set").unwrap();
                self.builder.build_call(f, &[list_ptr.into(), key.into(), val.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(None)
            }

            ("__HashMap", "getOrDefault") => {
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };

                let def = if let Some(a) = args.get(1) {
                    self.compile_expr(a)?.map(|v| self.value_to_i64(v)).transpose()?
                        .unwrap_or_else(|| i64_ty.const_int(0, false))
                } else { i64_ty.const_int(0, false) };

                let f = self.module.get_function("arc_map_get_or_default").unwrap();
                let r = self.builder.build_call(
                    f, &[list_ptr.into(), key.into(), def.into()], "map_val"
                ).map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            ("__HashMap", "get") => {
                // get → nullable: default olarak 0 döndür (null semantiği yok henüz)
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };
                let def = i64_ty.const_int(0, false);
                let f = self.module.get_function("arc_map_get_or_default").unwrap();
                let r = self.builder.build_call(
                    f, &[list_ptr.into(), key.into(), def.into()], "map_val"
                ).map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            ("__HashMap", "containsKey") => {
                for a in args { self.compile_expr(a)?; }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            ("__HashMap", "length") => {
                Ok(Some(i64_ty.const_int(0, false).into()))
            }

            ("__HashMap", "keys") | ("__HashMap", "values") | ("__HashMap", "entries")
            | ("__HashMap", "remove") => {
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }

            // ── Pair metodları ───────────────────────────────────────────────
            ("__Pair", "getFirst") => {
                let f = self.module.get_function("arc_pair_first").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into()], "pair_fst")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                // i64 → ptr (String ise) : inttoptr
                if let Some(BasicValueEnum::IntValue(v)) = r.try_as_basic_value().basic() {
                    let ptr = self.builder.build_int_to_ptr(v, ptr_ty, "fst_ptr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(ptr.into()));
                }
                Ok(r.try_as_basic_value().basic())
            }

            ("__Pair", "getSecond") => {
                let f = self.module.get_function("arc_pair_second").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into()], "pair_snd")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            _ => {
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }
        }
    }

    // ── Lambda → LLVM function pointer (filter için) ──────────────────────────

    fn compile_lambda_for_filter(
        &mut self,
        lambda   : &Expr,
        elem_cls : Option<String>,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let (params, body) = match lambda {
            Expr::Lambda { params, body } => (params.clone(), body.clone()),
            _ => return Ok(None),
        };

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let i1_ty  = self.ctx.bool_type();

        // Benzersiz lambda fonksiyon adı
        self.lambda_counter += 1;
        let fn_name = format!("arc_lambda_{}", self.lambda_counter);

        // Fonksiyon tipi: i64(i64) — item'ı i64 olarak alır, bool (i64) döner
        let fn_ty = i64_ty.fn_type(&[i64_ty.into()], false);
        let fn_val = self.module.add_function(&fn_name, fn_ty, None);

        // Builder konumunu kaydet
        let prev_block = self.builder.get_insert_block();
        let prev_fn = self.cur_fn;
        let prev_class = self.cur_class.clone();

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        self.cur_fn = Some(fn_val);
        self.push_scope();

        // Parametre: i64 → ptr (item pointer)
        let param_name = params.first().map(|s| s.as_str()).unwrap_or("item");
        let item_i64 = fn_val.get_nth_param(0).unwrap().into_int_value();

        // i64 → ptr dönüşümü
        let item_ptr = self.builder.build_int_to_ptr(item_i64, ptr_ty, "item_ptr")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Lambda parametresini scope'a ekle
        let alloca = self.builder.build_alloca(ptr_ty, param_name)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(alloca, item_ptr)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // class_name ile define et (elem_class'tan)
        let ec = elem_cls.clone();
        self.define_collection_var(param_name, alloca, ptr_ty.into(), ec.clone(), None);

        // cur_class'ı geçici olarak set et (method dispatch için)
        if let Some(ref cls) = ec {
            self.cur_class = Some(cls.clone());
        }

        // Body'yi derle
        let result = self.compile_expr(&body)?;

        // Sonucu i64'e genişlet (bool → i64)
        let ret_val = match result {
            Some(BasicValueEnum::IntValue(v)) => {
                if v.get_type().get_bit_width() == 1 {
                    self.builder.build_int_z_extend(v, i64_ty, "zext_ret")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                } else if v.get_type().get_bit_width() < 64 {
                    self.builder.build_int_z_extend(v, i64_ty, "zext_ret")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                } else {
                    v
                }
            }
            Some(BasicValueEnum::PointerValue(p)) => {
                self.builder.build_ptr_to_int(p, i64_ty, "ptr2i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
            }
            _ => i64_ty.const_int(0, false),
        };

        // Return değeri döndür
        let _ = i1_ty;
        self.builder.build_return(Some(&ret_val))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.pop_scope();
        self.cur_fn = prev_fn;
        self.cur_class = prev_class;

        // Builder konumunu geri yükle
        if let Some(bb) = prev_block {
            self.builder.position_at_end(bb);
        }

        fn_val.verify(true);

        // Fonksiyon pointer'ını döndür
        Ok(Some(fn_val.as_global_value().as_pointer_value().into()))
    }

    // ── ForEach döngüsü: List üzerinde ───────────────────────────────────────

    fn compile_for_each(
        &mut self,
        ty   : &Type,
        name : &str,
        iter : &Expr,
        body : &[Stmt],
    ) -> CgResult<bool> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();

        let iter_val = match self.compile_expr(iter)? {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(false),
        };

        let list_len_fn = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn = self.module.get_function("arc_list_get").unwrap();

        let len_call = self.builder.build_call(list_len_fn, &[iter_val.into()], "fe_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len_val = len_call.try_as_basic_value().basic()
            .ok_or_else(|| CodeGenError::new("arc_list_length returned void"))?;
        let len_i64 = match len_val { BasicValueEnum::IntValue(v) => v, _ => return Ok(false) };

        // Sayaç alloca
        let idx_alloca = self.builder.build_alloca(i64_ty, "fe_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cur_fn = self.cur_fn.unwrap();
        let cond_bb = self.ctx.append_basic_block(cur_fn, "fe.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "fe.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "fe.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Koşul: idx < len
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(BasicTypeEnum::IntType(i64_ty), idx_alloca, "fe_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(
            inkwell::IntPredicate::SLT, idx, len_i64, "fe_cond"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Gövde: item al, değişkene bağla, body'yi derle
        self.builder.position_at_end(body_bb);
        self.push_scope();

        let item_call = self.builder.build_call(
            list_get_fn, &[iter_val.into(), idx.into()], "fe_item"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;

        if let Some(BasicValueEnum::IntValue(item_i64)) = item_call.try_as_basic_value().basic() {
            // i64 → ptr (class instance)
            let item_ptr = self.builder.build_int_to_ptr(item_i64, ptr_ty, "fe_ptr")
                .map_err(|e| CodeGenError::new(e.to_string()))?;

            let alloca = self.builder.build_alloca(ptr_ty, name)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_store(alloca, item_ptr)
                .map_err(|e| CodeGenError::new(e.to_string()))?;

            // class_name'i Type'dan çıkar
            let class_name = match ty {
                Type::Named(n) => Some(n.clone()),
                _ => None,
            };
            self.define_collection_var(name, alloca, ptr_ty.into(), class_name, None);
        }

        for s in body {
            if self.compile_stmt(s)? { break; }
        }
        self.pop_scope();

        // idx++
        if !self.current_block_terminated() {
            let idx2 = self.builder.build_load(BasicTypeEnum::IntType(i64_ty), idx_alloca, "fe_i2")
                .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
            let idx3 = self.builder.build_int_add(idx2, i64_ty.const_int(1, false), "fe_inc")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_store(idx_alloca, idx3)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_unconditional_branch(cond_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.builder.position_at_end(exit_bb);
        Ok(false)
    }

    // ── Değer → i64 dönüşümü (koleksiyon item storage için) ──────────────────

    fn value_to_i64(
        &mut self,
        val : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::IntValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        match val {
            BasicValueEnum::IntValue(v) => {
                let w = v.get_type().get_bit_width();
                if w < 64 {
                    self.builder.build_int_z_extend(v, i64_ty, "zext64")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                } else if w > 64 {
                    self.builder.build_int_truncate(v, i64_ty, "trunc64")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                } else {
                    Ok(v)
                }
            }
            BasicValueEnum::PointerValue(p) => {
                self.builder.build_ptr_to_int(p, i64_ty, "ptr2i64")
                    .map_err(|e| CodeGenError::new(e.to_string()))
            }
            BasicValueEnum::FloatValue(f) => {
                self.builder.build_float_to_signed_int(f, i64_ty, "f2i64")
                    .map_err(|e| CodeGenError::new(e.to_string()))
            }
            _ => Ok(i64_ty.const_int(0, false)),
        }
    }

    // ── LLVM IR çıktısı ve native binary üretimi ─────────────────────────────

    pub fn emit_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn emit_object_file(&self, path: &Path) -> CgResult<()> {
        Target::initialize_x86(&InitializationConfig::default());

        let triple   = TargetMachine::get_default_triple();
        let target   = Target::from_triple(&triple)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let cpu      = TargetMachine::get_host_cpu_name();
        let features = TargetMachine::get_host_cpu_features();

        let machine = target.create_target_machine(
            &triple,
            cpu.to_str().unwrap_or("generic"),
            features.to_str().unwrap_or(""),
            OptimizationLevel::Default,
            RelocMode::Default,
            CodeModel::Default,
        ).ok_or_else(|| CodeGenError::new("could not create target machine"))?;

        machine.write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| CodeGenError::new(e.to_string()))
    }

    pub fn verify_module(&self) -> CgResult<()> {
        self.module.verify()
            .map_err(|e| CodeGenError::new(e.to_string()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Giriş noktası: .arm → .o üretimi
// ─────────────────────────────────────────────────────────────────────────────

pub fn emit_ir_only(
    module_ast  : &crate::ast::Module,
    module_name : &str,
) -> Result<String, CodeGenError> {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, module_name);
    cg.compile_module(module_ast)?;
    Ok(cg.emit_ir())
}

pub fn compile_to_object(
    module_ast  : &crate::ast::Module,
    module_name : &str,
    out_path    : &Path,
) -> Result<(), CodeGenError> {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, module_name);
    cg.compile_module(module_ast)?;
    cg.verify_module()?;
    cg.emit_object_file(out_path)
}
