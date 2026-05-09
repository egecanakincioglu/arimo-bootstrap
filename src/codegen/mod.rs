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
    class_name : Option<String>,   // class instance değişkenleri için
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
            scope.insert(name.to_string(), VarSlot { ptr, ty, class_name });
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

        // printf + malloc her zaman lazım
        self.declare_printf();
        self.declare_malloc();

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
        // Sadece instance field'ları (static değil)
        let field_types: Vec<BasicTypeEnum<'ctx>> = c.fields.iter()
            .filter(|f| !f.static_)
            .filter_map(|f| self.llvm_type(&f.ty))
            .collect();

        let struct_ty = self.ctx.struct_type(&field_types, false);
        self.struct_types.insert(c.name.clone(), struct_ty);

        // Field adı → indeks eşlemesi
        let mut idx_map = HashMap::new();
        let mut idx = 0u32;
        for f in c.fields.iter().filter(|f| !f.static_) {
            idx_map.insert(f.name.clone(), idx);
            idx += 1;
        }
        self.field_indices.insert(c.name.clone(), idx_map);
    }

    // ── Enum kayıt ───────────────────────────────────────────────────────────

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
                // Class instance için class adını kaydet
                let class_name = match ty {
                    Type::Named(n) if self.struct_types.contains_key(n.as_str()) => {
                        Some(n.clone())
                    }
                    _ => None,
                };

                // Enum tip → i32, class instance → pointer, diğerleri → direkt tip
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
                    self.define_var_with_class(name, alloca, llvm_ty, class_name);
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
                // ForEach: koleksiyonlar üzerinde — şimdilik pass
                let _ = (ty, name, iter, body);
                Ok(false)
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
                // String interpolation: şimdilik sadece ilk metin kısmını al
                let mut result = String::new();
                for part in parts {
                    match part {
                        StringPart::Text(t) => result.push_str(t),
                        StringPart::Interp(_) => result.push_str("%s"),
                    }
                }
                let g = self.build_global_string(&result)?;
                Ok(Some(g.into()))
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

            // ── Diğer static çağrılar ─────────────────────────────────────────
            Expr::StaticCall { class, method, args } => {
                self.compile_static_call(class, method, args)
            }

            // ── Instance metod çağrısı ─────────────────────────────────────────
            Expr::MethodCall { object, method, args } => {
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
                    // Struct tipi bilinen ama constructor'ı olmayan case
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
                // Enum.Variant → integer sabit
                if let Expr::Ident(enum_name) = object.as_ref() {
                    if let Some(val) = self.enum_variant_value(enum_name, field) {
                        let iv = self.ctx.i32_type().const_int(val as u64, false);
                        return Ok(Some(iv.into()));
                    }
                }
                self.compile_field_load(object, field)
            }
            Expr::NullSafeAccess { .. } => Ok(None),
        }
    }

    // ── IO.print() ───────────────────────────────────────────────────────────

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

    // Nesnenin class adını VarSlot'tan veya cur_class'tan çıkar
    fn infer_object_class(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::This => self.cur_class.clone(),
            Expr::Ident(name) => {
                for scope in self.scopes.iter().rev() {
                    if let Some(slot) = scope.get(name.as_str()) {
                        // VarSlot'ta class adı saklıysa onu kullan
                        if slot.class_name.is_some() {
                            return slot.class_name.clone();
                        }
                    }
                }
                None
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
                // this.field = value
                Expr::FieldAccess { object, field } => {
                    let class_name = self.infer_object_class(object);
                    let obj_ptr = self.compile_expr(object)?;
                    if let (Some(cn), Some(BasicValueEnum::PointerValue(ptr))) = (class_name, obj_ptr) {
                        self.gep_field_store(&cn.clone(), ptr, field, v)?;
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
