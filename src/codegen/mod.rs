/*
Arimo Lang - A modern programming language and compiler
Copyright (C) 2026 Egecan Akıncıoğlu

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published
by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/


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
    // Break/continue hedefleri (döngü yığını)
    loop_exit_bbs     : Vec<inkwell::basic_block::BasicBlock<'ctx>>,
    loop_continue_bbs : Vec<inkwell::basic_block::BasicBlock<'ctx>>,
    // ARC: class adı → struct içinde refcount field indeksi
    refcount_indices     : HashMap<String, u32>,
    // ARC: @ManualMemory class'ları (ARC skip edilir)
    manual_memory_classes: std::collections::HashSet<String>,
    // Exception: finally garantisi — return öncesinde çalıştırılacak body'ler
    finally_defers       : Vec<Vec<Stmt>>,
    // Defer: scope çıkışında (LIFO) çalıştırılacak expression'lar
    defer_stack          : Vec<Vec<Expr>>,
    // EH: her try bloğunda kaydedilen eski @__arimo_ex_top alloca'ları
    // [0] = bu fonksiyondaki ilk try'dan önceki top (return sırasında kullanılır)
    try_saved_tops       : Vec<inkwell::values::PointerValue<'ctx>>,
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
            loop_exit_bbs         : Vec::new(),
            loop_continue_bbs     : Vec::new(),
            refcount_indices      : HashMap::new(),
            manual_memory_classes : std::collections::HashSet::new(),
            finally_defers        : Vec::new(),
            defer_stack           : Vec::new(),
            try_saved_tops        : Vec::new(),
        }
    }

    // ── Kapsam yönetimi ──────────────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.defer_stack.push(Vec::new());
    }

    // ── ARC: retain (refcount++) ──────────────────────────────────────────────

    fn arc_retain_ptr(
        &mut self,
        ptr        : inkwell::values::PointerValue<'ctx>,
        class_name : &str,
    ) -> CgResult<()> {
        if self.manual_memory_classes.contains(class_name) { return Ok(()); }
        if !self.refcount_indices.contains_key(class_name)  { return Ok(()); }
        if self.current_block_terminated()                   { return Ok(()); }
        let cur_fn = match self.cur_fn { Some(f) => f, None => return Ok(()) };

        let i64_ty  = self.ctx.i64_type();
        let inc_bb  = self.ctx.append_basic_block(cur_fn, "arc.inc");
        let cont_bb = self.ctx.append_basic_block(cur_fn, "arc.ri_cont");

        // Null guard
        let pi      = self.builder.build_ptr_to_int(ptr, i64_ty, "arc_ri_pi")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let is_null = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, pi, i64_ty.const_int(0, false), "arc_ri_null"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(is_null, cont_bb, inc_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(inc_bb);
        let rc_idx    = *self.refcount_indices.get(class_name).unwrap();
        let struct_ty = *self.struct_types.get(class_name).unwrap();
        let rc_gep    = self.builder.build_struct_gep(struct_ty, ptr, rc_idx, "arc_ri_gep")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let rc_old    = self.builder.build_load(i64_ty, rc_gep, "arc_ri_rc")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let rc_new    = self.builder.build_int_add(rc_old, i64_ty.const_int(1, false), "arc_ri_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(rc_gep, rc_new)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cont_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    // Expression'dan class adını çıkarmaya çalış (retain için)
    fn infer_expr_class_name(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => self.lookup_var(name)
                .and_then(|s| s.class_name.clone())
                .filter(|cn| self.struct_types.contains_key(cn.as_str())),
            _ => None,
        }
    }

    // ── ARC: retain/release + all-scopes helpers ──────────────────────────────

    // Belirli bir değişken adı HARİÇ tüm scope'ları release et (Return için)
    fn arc_release_all_scopes_except(&mut self, skip_var: Option<&str>) -> CgResult<()> {
        for i in (0..self.scopes.len()).rev() {
            if self.current_block_terminated() { break; }
            let slots: Vec<(String, VarSlot<'ctx>)> = self.scopes[i]
                .iter()
                .filter(|(k, s)| {
                    s.class_name.is_some()
                        && skip_var.map_or(true, |skip| k.as_str() != skip)
                })
                .map(|(k, s)| (k.clone(), s.clone()))
                .collect();
            for (_, slot) in slots {
                if self.current_block_terminated() { break; }
                self.arc_release_var(slot)?;
            }
        }
        Ok(())
    }

    // ── ARC: tek değişken için inline release emit ────────────────────────────

    fn arc_release_var(&mut self, slot: VarSlot<'ctx>) -> CgResult<()> {
        let class_name = match &slot.class_name {
            Some(n) if !self.manual_memory_classes.contains(n.as_str())
                    && self.refcount_indices.contains_key(n.as_str()) => n.clone(),
            _ => return Ok(()),
        };
        if self.current_block_terminated() { return Ok(()); }
        let cur_fn = match self.cur_fn { Some(f) => f, None => return Ok(()) };

        // Pointer'ı yükle
        let ptr_val = self.builder.build_load(slot.ty, slot.ptr, "arc_ptr")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ptr = match ptr_val {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Ok(()),
        };

        let i64_ty  = self.ctx.i64_type();
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let dec_bb  = self.ctx.append_basic_block(cur_fn, "arc.dec");
        let free_bb = self.ctx.append_basic_block(cur_fn, "arc.free");
        let cont_bb = self.ctx.append_basic_block(cur_fn, "arc.cont");

        // Null check: null ise atla
        let ptr_int  = self.builder.build_ptr_to_int(ptr, i64_ty, "arc_pi")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let is_null  = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, ptr_int, i64_ty.const_int(0, false), "arc_null"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(is_null, cont_bb, dec_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Decrement block
        self.builder.position_at_end(dec_bb);
        let rc_idx    = *self.refcount_indices.get(&class_name).unwrap();
        let struct_ty = *self.struct_types.get(&class_name).unwrap();
        let rc_gep    = self.builder.build_struct_gep(struct_ty, ptr, rc_idx, "arc_rc_gep")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let rc_old    = self.builder.build_load(i64_ty, rc_gep, "arc_rc")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let rc_new    = self.builder.build_int_sub(rc_old, i64_ty.const_int(1, false), "arc_rc_dec")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(rc_gep, rc_new)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let is_zero   = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, rc_new, i64_ty.const_int(0, false), "arc_zero"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(is_zero, free_bb, cont_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Free block
        self.builder.position_at_end(free_bb);
        let free_fn = self.module.get_function("free").unwrap_or_else(|| {
            let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
            self.module.add_function("free", ft, None)
        });
        self.builder.build_call(free_fn, &[ptr.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cont_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    // Belirli bir scope indeksindeki class değişkenlerini serbest bırak
    fn arc_release_scope(&mut self, scope_idx: usize) -> CgResult<()> {
        let slots: Vec<VarSlot<'ctx>> = self.scopes[scope_idx]
            .values()
            .filter(|s| s.class_name.is_some())
            .cloned()
            .collect();
        for slot in slots {
            if self.current_block_terminated() { break; }
            self.arc_release_var(slot)?;
        }
        Ok(())
    }

    // Return öncesinde TÜM scope'lardaki class değişkenlerini serbest bırak
    fn arc_release_all_scopes(&mut self) -> CgResult<()> {
        for i in (0..self.scopes.len()).rev() {
            if self.current_block_terminated() { break; }
            self.arc_release_scope(i)?;
        }
        Ok(())
    }

    fn pop_scope(&mut self) {
        // Defer: scope çıkışında kayıtlı expression'ları LIFO çalıştır
        if !self.current_block_terminated() {
            let defers = self.defer_stack.last().cloned().unwrap_or_default();
            for expr in defers.iter().rev() {
                if self.current_block_terminated() { break; }
                let _ = self.compile_expr(expr);
            }
        }
        self.defer_stack.pop();

        // ARC: scope çıkışında class instance'ları serbest bırak
        if let Some(idx) = self.scopes.len().checked_sub(1) {
            if !self.current_block_terminated() {
                let _ = self.arc_release_scope(idx);
            }
        }
        self.scopes.pop();
    }

    fn pop_scope_no_arc(&mut self) {
        self.defer_stack.pop();
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
            // SIMD vektör tipleri
            Type::Named(n) if n == "Vec4f" => Some(self.ctx.f32_type().vec_type(4).into()),
            Type::Named(n) if n == "Vec8f" => Some(self.ctx.f32_type().vec_type(8).into()),
            Type::Named(n) if n == "Vec4i" => Some(self.ctx.i32_type().vec_type(4).into()),
            Type::Named(n) if n == "Vec8i" => Some(self.ctx.i32_type().vec_type(8).into()),
            Type::Named(_) => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::FnPtr(_, _) => {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                Some(self.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false).into())
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

        // Geçiş 0: @ManualMemory class'larını topla (ARC skip için)
        for item in &module.items {
            if let Item::Class(c) = item {
                if c.manual {
                    self.manual_memory_classes.insert(c.name.clone());
                }
            }
        }

        // Geçiş 0b: enum variant'larını kayıt et
        for item in &module.items {
            if let Item::Enum(e) = item {
                self.register_enum(e);
            }
        }
        // Geçiş 1: struct/class tiplerini kayıt et (field layout)
        for item in &module.items {
            match item {
                Item::Class(c) => self.register_class_struct(c),
                Item::Struct(s) => self.register_struct_decl(s),
                _ => {}
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

    // ── Inline asm ───────────────────────────────────────────────────────────
    // LLVM inline asm: void asm sideeffect "...", ""()
    fn compile_asm(&mut self, code: &str) -> CgResult<()> {
        let void_ty = self.ctx.void_type();
        let fn_ty   = void_ty.fn_type(&[], false);
        // sanitize: replace \n with actual newline for LLVM asm string
        let asm_code = code.replace("\\n", "\n").replace("\\t", "\t");
        let inline_asm = self.ctx.create_inline_asm(
            fn_ty,
            asm_code,
            String::new(),
            true,  // has_side_effects
            false, // is_align_stack
            None,  // dialect (None = AT&T)
            false, // can_throw
        );
        self.builder.build_indirect_call(fn_ty, inline_asm, &[], "asm_call")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
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

    // ── EH runtime tanımları (setjmp/longjmp tabanlı) ────────────────────────

    fn declare_setjmp(&mut self) {
        // Windows UCRT: _setjmp(jmp_buf, void* frame) — 2 args
        // Linux/macOS:  setjmp(jmp_buf) — 1 arg
        #[cfg(target_os = "windows")]
        let fn_name = "_setjmp";
        #[cfg(not(target_os = "windows"))]
        let fn_name = "setjmp";

        if self.module.get_function(fn_name).is_some() { return; }
        let i32_ty = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        #[cfg(target_os = "windows")]
        let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        #[cfg(not(target_os = "windows"))]
        let ft = i32_ty.fn_type(&[ptr_ty.into()], false);
        let f = self.module.add_function(fn_name, ft, None);
        let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("returns_twice");
        if kind_id != 0 {
            let attr = self.ctx.create_enum_attribute(kind_id, 0);
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
        }
    }

    fn setjmp_fn_name(&self) -> &'static str {
        #[cfg(target_os = "windows")] { "_setjmp" }
        #[cfg(not(target_os = "windows"))] { "setjmp" }
    }

    fn declare_longjmp(&mut self) {
        if self.module.get_function("longjmp").is_some() { return; }
        let void_ty = self.ctx.void_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32_ty = self.ctx.i32_type();
        let ft = void_ty.fn_type(&[ptr_ty.into(), i32_ty.into()], false);
        let f = self.module.add_function("longjmp", ft, None);
        let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
        if kind_id != 0 {
            let attr = self.ctx.create_enum_attribute(kind_id, 0);
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
        }
    }

    fn declare_strcmp(&mut self) {
        if self.module.get_function("strcmp").is_some() { return; }
        let i32_ty = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        self.module.add_function("strcmp", ft, None);
    }

    // EH global'ı al ya da oluştur (@__arimo_ex_top, @__arimo_ex_type, @__arimo_ex_msg)
    fn get_or_create_eh_global(&mut self, name: &str) -> inkwell::values::GlobalValue<'ctx> {
        if let Some(g) = self.module.get_global(name) { return g; }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let g = self.module.add_global(ptr_ty, None, name);
        g.set_initializer(&ptr_ty.const_null());
        g.set_linkage(inkwell::module::Linkage::Internal);
        g
    }

    // EH: global jmpbuf dizisi ve derinlik sayacı
    // Stack alloca yerine global kullanmak alignment sorununu çözer
    // [32 x [32 x i64]] = 32 iç içe try seviyesi, her biri 256 byte
    fn get_or_create_eh_jmpbufs(&mut self) -> inkwell::values::GlobalValue<'ctx> {
        if let Some(g) = self.module.get_global("__arimo_ex_jmpbufs") { return g; }
        let i64_ty = self.ctx.i64_type();
        let jmpbuf_ty = i64_ty.array_type(32);   // [32 x i64] = 256 bytes per slot
        let arr_ty = jmpbuf_ty.array_type(32);    // [32 x [32 x i64]] = 32 nesting levels
        let g = self.module.add_global(arr_ty, None, "__arimo_ex_jmpbufs");
        g.set_initializer(&arr_ty.const_zero());
        g.set_linkage(inkwell::module::Linkage::Internal);
        g.set_alignment(32);  // 32-byte aligned for XMM registers (UCRT requirement)
        g
    }

    fn get_or_create_eh_depth(&mut self) -> inkwell::values::GlobalValue<'ctx> {
        if let Some(g) = self.module.get_global("__arimo_ex_depth") { return g; }
        let i32_ty = self.ctx.i32_type();
        let g = self.module.add_global(i32_ty, None, "__arimo_ex_depth");
        g.set_initializer(&i32_ty.const_int(0, false));
        g.set_linkage(inkwell::module::Linkage::Internal);
        g
    }

    // slot indeksinden jmpbuf pointer'ı üret
    fn get_jmpbuf_ptr(&mut self, slot: inkwell::values::IntValue<'ctx>) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let jmpbuf_slot_ty = i64_ty.array_type(32);  // [32 x i64]
        let arr_ty = jmpbuf_slot_ty.array_type(32);  // [32 x [32 x i64]]
        let jmpbufs_gv = self.get_or_create_eh_jmpbufs();
        let i32_ty = self.ctx.i32_type();
        let zero = i32_ty.const_int(0, false);
        let ptr = unsafe {
            self.builder.build_gep(arr_ty, jmpbufs_gv.as_pointer_value(), &[zero, slot], "jmpbuf_slot")
        }.map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(ptr)
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
                    }
                    // Child field'ları parent'ın TÜM field'larından (refcount dahil) sonra başlar
                    idx = parent_struct.count_fields() as u32;
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

        // ARC: refcount field'ı ekle (kalıtımda parent'ın refcount'u yeniden kullanılır)
        let rc_idx = if let Some(parent) = &c.extends {
            // Parent'ın refcount indeksini miras al
            self.refcount_indices.get(parent.as_str()).copied().unwrap_or_else(|| {
                let i = all_field_types.len() as u32;
                all_field_types.push(self.ctx.i64_type().into());
                i
            })
        } else {
            // Baz sınıf: sona refcount ekle
            let i = all_field_types.len() as u32;
            all_field_types.push(self.ctx.i64_type().into());
            i
        };
        self.refcount_indices.insert(c.name.clone(), rc_idx);

        let struct_ty = self.ctx.struct_type(&all_field_types, false); // ClassDecl: never packed
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

    // ── StructDecl kayıt (@Packed, @Align desteğiyle) ────────────────────────
    fn register_struct_decl(&mut self, s: &StructDecl) {
        let mut field_types: Vec<BasicTypeEnum<'ctx>> = Vec::new();
        let mut idx_map: HashMap<String, u32> = HashMap::new();

        for (i, f) in s.fields.iter().enumerate() {
            if let Some(ft) = self.llvm_type(&f.ty) {
                field_types.push(ft);
                idx_map.insert(f.name.clone(), i as u32);
            }
        }

        // @Packed → packed=true
        let struct_ty = self.ctx.struct_type(&field_types, s.packed);
        self.struct_types.insert(s.name.clone(), struct_ty);
        self.field_indices.insert(s.name.clone(), idx_map);
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

        // ARC: refcount = 1 başlat
        if !self.manual_memory_classes.contains(&c.name) {
            if let Some(&rc_idx) = self.refcount_indices.get(&c.name) {
                if let BasicValueEnum::PointerValue(ptr) = obj_ptr {
                    let rc_gep = self.builder.build_struct_gep(struct_ty, ptr, rc_idx, "rc_init_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(rc_gep, self.ctx.i64_type().const_int(1, false))
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
            }
        }

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

        // ── LLVM Attribute'ları uygula ────────────────────────────────────────
        // noreturn → LLVM noreturn attribute
        if m.return_ty.as_ref().map(|t| matches!(t, Type::NoReturn)).unwrap_or(false) {
            let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
            if kind_id != 0 {
                let attr = self.ctx.create_enum_attribute(kind_id, 0);
                fn_val.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
            }
        }
        // @ForceInline → alwaysinline
        if m.inline_ {
            let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("alwaysinline");
            if kind_id != 0 {
                let attr = self.ctx.create_enum_attribute(kind_id, 0);
                fn_val.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
            }
        }
        // @Pure → readnone
        if m.pure_ {
            let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("readnone");
            if kind_id != 0 {
                let attr = self.ctx.create_enum_attribute(kind_id, 0);
                fn_val.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
            }
        }
        // @Section("name") → section attribute
        if let Some(section) = &m.section {
            fn_val.set_section(Some(section.as_str()));
        }
        // @CallingConvention → LLVM calling conv
        if let Some(cc) = &m.calling_conv {
            let llvm_cc = match cc {
                CallingConv::Cdecl    => inkwell::llvm_sys::LLVMCallConv::LLVMCCallConv as u32,
                CallingConv::Stdcall  => 64u32, // stdcall = 64
                CallingConv::Interrupt => 86u32, // x86_intr = 86
            };
            fn_val.set_call_conventions(llvm_cc);
        }

        let entry_block = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry_block);

        self.cur_fn = Some(fn_val);
        self.try_saved_tops.clear();
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

        // Return yoksa otomatik ekle — önce ARC cleanup (function scope)
        if !returned {
            if !self.current_block_terminated() {
                if let Some(idx) = self.scopes.len().checked_sub(1) {
                    self.arc_release_scope(idx)?;
                }
            }
            if is_entry {
                let zero = self.ctx.i32_type().const_int(0, false);
                self.builder.build_return(Some(&zero))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }
        }

        self.pop_scope_no_arc(); // block sonlandı, ARC zaten yapıldı
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
                // ── Adım 1: Return değerini ÖNCE derle ──────────────────────
                // (Cleanup'tan önce değerlendirmek use-after-free'yi önler)
                let (ret_val, skip_var_name) = match expr {
                    None => (None, None),
                    Some(e) => {
                        // Döndürülen değişken adını bul (ARC'da bu değişkeni skip edeceğiz)
                        let skip = match e {
                            Expr::Ident(n) => Some(n.clone()),
                            _ => None,
                        };
                        let val  = self.compile_expr(e)?;
                        (val, skip)
                    }
                };

                // ── Adım 2: Finally defers çalıştır ─────────────────────────
                let defers = self.finally_defers.clone();
                for fin_body in defers.iter().rev() {
                    if self.current_block_terminated() { break; }
                    self.push_scope();
                    for s in fin_body { if self.compile_stmt(s)? { break; } }
                    self.pop_scope();
                }

                // ── Adım 2b: EH depth'i temizle (try içinde return varsa) ──────
                if !self.try_saved_tops.is_empty() && !self.current_block_terminated() {
                    let i32_ty = self.ctx.i32_type();
                    let depth_gv = self.get_or_create_eh_depth();
                    let outermost_depth_alloca = self.try_saved_tops[0];
                    if let Ok(saved) = self.builder.build_load(i32_ty, outermost_depth_alloca, "ret_eh_depth") {
                        let _ = self.builder.build_store(depth_gv.as_pointer_value(), saved);
                    }
                }

                // ── Adım 3: ARC cleanup — döndürülen değişkeni atla ─────────
                // (double-free'yi önler: döndürülen nesne caller'a transfer edilir)
                self.arc_release_all_scopes_except(skip_var_name.as_deref())?;

                // ── Adım 4: Return IR emit ───────────────────────────────────
                match ret_val {
                    None => {
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
                    Some(v) => {
                        self.builder.build_return(Some(&v))
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                }
                Ok(true)
            }

            Stmt::VarDecl { ty, name, value, volatile, .. } => {
                let is_volatile = *volatile;
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
                            let store = self.builder.build_store(alloca, coerced)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            if is_volatile { let _ = store.set_volatile(true); }

                            // ARC retain:
                            // - ConstructorCall → yapıcı zaten refcount=1 verir → retain YOK
                            // - StaticCall/MethodCall → callee +1 döndürür → retain YOK
                            // - Ident → mevcut nesneyi kopyalıyoruz → retain GEREKLİ
                            // - Diğer (FieldAccess vs.) → retain yapılmaz (sınır: gelecekte eklenecek)
                            if let Some(cn) = &class_name {
                                if !matches!(cn.as_str(), "__List" | "__HashMap" | "__Pair") {
                                    let needs_retain = matches!(init_expr, Expr::Ident(_));
                                    if needs_retain {
                                        if let BasicValueEnum::PointerValue(ptr) = coerced {
                                            let cn_owned = cn.clone();
                                            self.arc_retain_ptr(ptr, &cn_owned)?;
                                        }
                                    }
                                }
                            }
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

            Stmt::If { hint, cond, then, else_if, else_ } => {
                self.compile_if(hint.as_ref(), cond, then, else_if, else_)
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

            Stmt::TryCatch { try_body, catches, finally_body } => {
                self.declare_setjmp();
                self.declare_longjmp();
                self.declare_strcmp();

                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i32_ty = self.ctx.i32_type();
                let cur_fn = self.cur_fn.unwrap();

                // ── 1. Mevcut derinliği oku (saved_depth = try öncesi derinlik) ──
                let depth_gv = self.get_or_create_eh_depth();
                let saved_depth = self.builder.build_load(i32_ty, depth_gv.as_pointer_value(), "saved_depth")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .into_int_value();

                // saved_depth'i bir alloca'ya kaydet (returns_twice sonrası erişim için)
                let saved_depth_alloca = self.builder.build_alloca(i32_ty, "saved_depth_slot")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(saved_depth_alloca, saved_depth)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.try_saved_tops.push(saved_depth_alloca
                    .as_instruction_value().map(|_| saved_depth_alloca)
                    .unwrap_or(saved_depth_alloca));

                // ── 2. Derinliği artır ──────────────────────────────────────────
                let new_depth = self.builder.build_int_add(saved_depth, i32_ty.const_int(1, false), "new_depth")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(depth_gv.as_pointer_value(), new_depth)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                // ── 3. Bu try için jmpbuf pointer'ı al ─────────────────────────
                let jmpbuf_ptr = self.get_jmpbuf_ptr(saved_depth)?;

                // ── 4. setjmp/longjmp çağrısı ──────────────────────────────────
                let sj_name = self.setjmp_fn_name();
                let setjmp_fn = self.module.get_function(sj_name).unwrap();
                #[cfg(target_os = "windows")]
                let setjmp_args: &[inkwell::values::BasicMetadataValueEnum] = &[
                    jmpbuf_ptr.into(),
                    self.ctx.ptr_type(inkwell::AddressSpace::default()).const_null().into(),
                ];
                #[cfg(not(target_os = "windows"))]
                let setjmp_args: &[inkwell::values::BasicMetadataValueEnum] = &[jmpbuf_ptr.into()];
                let setjmp_r = self.builder.build_call(setjmp_fn, setjmp_args, "setjmp_r")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .try_as_basic_value().basic().unwrap().into_int_value();

                // ── 5. Branch: 0 → try body, nonzero → catch dispatch ───────────
                let try_body_bb   = self.ctx.append_basic_block(cur_fn, "try.body");
                let catch_disp_bb = self.ctx.append_basic_block(cur_fn, "catch.dispatch");
                let finally_bb    = self.ctx.append_basic_block(cur_fn, "try.finally");
                let after_bb      = self.ctx.append_basic_block(cur_fn, "try.after");

                let is_ex = self.builder.build_int_compare(
                    inkwell::IntPredicate::NE, setjmp_r, i32_ty.const_int(0, false), "is_ex")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_conditional_branch(is_ex, catch_disp_bb, try_body_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                // ── Try body ────────────────────────────────────────────────────
                self.builder.position_at_end(try_body_bb);

                if let Some(fin) = finally_body {
                    self.finally_defers.push(fin.clone());
                }

                let mut try_returned = false;
                self.push_scope();
                for s in try_body {
                    if self.compile_stmt(s)? { try_returned = true; break; }
                }
                self.pop_scope();

                if finally_body.is_some() { self.finally_defers.pop(); }

                // Normal çıkış: derinliği geri yükle → finally
                if !try_returned && !self.current_block_terminated() {
                    let sd = self.builder.build_load(i32_ty, saved_depth_alloca, "sd_restore")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(depth_gv.as_pointer_value(), sd)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unconditional_branch(finally_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                // ── Catch dispatch ───────────────────────────────────────────────
                self.builder.position_at_end(catch_disp_bb);

                // Exception type ve message'ı yükle
                let ex_type_gv = self.get_or_create_eh_global("__arimo_ex_type");
                let ex_msg_gv  = self.get_or_create_eh_global("__arimo_ex_msg");
                let ex_type_val = self.builder.build_load(ptr_ty, ex_type_gv.as_pointer_value(), "ex_type")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let ex_msg_val = self.builder.build_load(ptr_ty, ex_msg_gv.as_pointer_value(), "ex_msg")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                let rethrow_bb = self.ctx.append_basic_block(cur_fn, "catch.rethrow");
                let strcmp_fn  = self.module.get_function("strcmp").unwrap();

                for (i, catch) in catches.iter().enumerate() {
                    let catch_body_bb = self.ctx.append_basic_block(cur_fn, &format!("catch.{}.body", i));
                    let next_bb = if i + 1 < catches.len() {
                        self.ctx.append_basic_block(cur_fn, &format!("catch.{}.check", i + 1))
                    } else {
                        rethrow_bb
                    };

                    let catch_type = match &catch.exception_type {
                        crate::ast::Type::Named(n) => n.clone(),
                        _ => "Exception".to_string(),
                    };

                    if catch_type == "Exception" {
                        self.builder.build_unconditional_branch(catch_body_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    } else {
                        let catch_type_ptr = self.build_global_string(&catch_type)?;
                        let cmp = self.builder.build_call(
                            strcmp_fn, &[ex_type_val.into(), catch_type_ptr.into()], "type_cmp")
                            .map_err(|e| CodeGenError::new(e.to_string()))?
                            .try_as_basic_value().basic().unwrap().into_int_value();
                        let matched = self.builder.build_int_compare(
                            inkwell::IntPredicate::EQ, cmp, i32_ty.const_int(0, false), "type_match")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_conditional_branch(matched, catch_body_bb, next_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }

                    self.builder.position_at_end(catch_body_bb);
                    self.push_scope();

                    if !catch.name.is_empty() {
                        let alloca = self.builder.build_alloca(ptr_ty, &catch.name)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        if ex_msg_val.is_pointer_value() {
                            self.builder.build_store(alloca, ex_msg_val.into_pointer_value())
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                        self.define_var(&catch.name, alloca, ptr_ty.into());
                    }

                    let mut catch_ret = false;
                    for s in &catch.body {
                        if self.compile_stmt(s)? { catch_ret = true; break; }
                    }
                    self.pop_scope();

                    if !catch_ret && !self.current_block_terminated() {
                        // Catch çıkışı: derinliği geri yükle → finally
                        let sd = self.builder.build_load(i32_ty, saved_depth_alloca, "sd_catch")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_store(depth_gv.as_pointer_value(), sd)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_unconditional_branch(finally_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }

                    if i + 1 < catches.len() {
                        self.builder.position_at_end(next_bb);
                    }
                }

                // Hiçbir catch uymadı → rethrow
                self.builder.position_at_end(rethrow_bb);
                {
                    // Derinliği saved_depth'e geri yükle
                    let sd = self.builder.build_load(i32_ty, saved_depth_alloca, "sd_rethrow")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                        .into_int_value();
                    self.builder.build_store(depth_gv.as_pointer_value(), sd)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;

                    // Parent slot = saved_depth - 1
                    let rethrow_null_bb    = self.ctx.append_basic_block(cur_fn, "rethrow.uncaught");
                    let rethrow_longjmp_bb = self.ctx.append_basic_block(cur_fn, "rethrow.longjmp");

                    let has_parent = self.builder.build_int_compare(
                        inkwell::IntPredicate::SGT, sd, i32_ty.const_int(0, false), "has_parent")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_conditional_branch(has_parent, rethrow_longjmp_bb, rethrow_null_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;

                    self.builder.position_at_end(rethrow_null_bb);
                    self.declare_printf();
                    let fmt = self.build_global_string("Uncaught exception: %s\n")?;
                    let printf = self.module.get_function("printf").unwrap();
                    let ex_type2 = self.builder.build_load(ptr_ty, ex_type_gv.as_pointer_value(), "ex_t2")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_call(printf, &[fmt.into(), ex_type2.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let abort_fn = self.module.get_function("abort").unwrap_or_else(|| {
                        let ft = self.ctx.void_type().fn_type(&[], false);
                        self.module.add_function("abort", ft, None)
                    });
                    self.builder.build_call(abort_fn, &[], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unreachable()
                        .map_err(|e| CodeGenError::new(e.to_string()))?;

                    self.builder.position_at_end(rethrow_longjmp_bb);
                    let parent_slot = self.builder.build_int_sub(sd, i32_ty.const_int(1, false), "parent_slot")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let parent_jmpbuf = self.get_jmpbuf_ptr(parent_slot)?;
                    let longjmp_fn = self.module.get_function("longjmp").unwrap();
                    let one = i32_ty.const_int(1, false);
                    self.builder.build_call(longjmp_fn, &[parent_jmpbuf.into(), one.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unreachable()
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                // ── Finally bloğu ────────────────────────────────────────────────
                self.builder.position_at_end(finally_bb);
                if let Some(fin) = finally_body {
                    self.push_scope();
                    for s in fin { if self.compile_stmt(s)? { break; } }
                    self.pop_scope();
                }
                if !self.current_block_terminated() {
                    self.builder.build_unconditional_branch(after_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                self.builder.position_at_end(after_bb);
                self.try_saved_tops.pop();

                Ok(false)
            }

            Stmt::Throw(expr) => {
                self.declare_setjmp();
                self.declare_longjmp();
                self.declare_printf();

                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i32_ty = self.ctx.i32_type();

                // Exception type adını ve mesaj değerini çıkart
                let (type_name_str, msg_opt) = match expr {
                    Expr::ConstructorCall { class, args, .. } => {
                        let name = class.clone();
                        let msg = if let Some(first) = args.first() {
                            self.compile_expr(first)?
                        } else {
                            None
                        };
                        (name, msg)
                    }
                    _ => ("Exception".to_string(), None),
                };

                // @__arimo_ex_type'a type name yaz
                let type_name_ptr = self.build_global_string(&type_name_str)?;
                let ex_type_gv = self.get_or_create_eh_global("__arimo_ex_type");
                self.builder.build_store(ex_type_gv.as_pointer_value(), type_name_ptr)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                // @__arimo_ex_msg'a mesajı yaz
                let ex_msg_gv = self.get_or_create_eh_global("__arimo_ex_msg");
                if let Some(msg_val) = msg_opt {
                    if msg_val.is_pointer_value() {
                        self.builder.build_store(ex_msg_gv.as_pointer_value(), msg_val.into_pointer_value())
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                } else {
                    let empty = self.build_global_string("")?;
                    self.builder.build_store(ex_msg_gv.as_pointer_value(), empty)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                // Derinliği yükle — 0 ise uncaught, >0 ise longjmp
                let depth_gv = self.get_or_create_eh_depth();
                let depth_val = self.builder.build_load(i32_ty, depth_gv.as_pointer_value(), "throw_depth")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .into_int_value();

                let cur_fn = self.cur_fn.unwrap();
                let do_longjmp_bb = self.ctx.append_basic_block(cur_fn, "throw.longjmp");
                let do_abort_bb   = self.ctx.append_basic_block(cur_fn, "throw.uncaught");

                let has_frame = self.builder.build_int_compare(
                    inkwell::IntPredicate::SGT, depth_val, i32_ty.const_int(0, false), "has_frame")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_conditional_branch(has_frame, do_longjmp_bb, do_abort_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                // ── Uncaught: mesajı yaz + abort ─────────────────────────────
                self.builder.position_at_end(do_abort_bb);
                {
                    let fmt = self.build_global_string("Uncaught exception %s: %s\n")?;
                    let printf = self.module.get_function("printf").unwrap();
                    let msg_reload = self.builder.build_load(ptr_ty, ex_msg_gv.as_pointer_value(), "msg_r")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_call(printf, &[fmt.into(), type_name_ptr.into(), msg_reload.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let abort_fn = self.module.get_function("abort").unwrap_or_else(|| {
                        let ft = self.ctx.void_type().fn_type(&[], false);
                        self.module.add_function("abort", ft, None)
                    });
                    self.builder.build_call(abort_fn, &[], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unreachable()
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                // ── longjmp: en içteki try frame'e zıpla ────────────────────
                self.builder.position_at_end(do_longjmp_bb);
                {
                    // slot = depth - 1 (0-indexed, innermost active slot)
                    let slot = self.builder.build_int_sub(depth_val, i32_ty.const_int(1, false), "throw_slot")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let jmpbuf_ptr = self.get_jmpbuf_ptr(slot)?;
                    let longjmp_fn = self.module.get_function("longjmp").unwrap();
                    let one = i32_ty.const_int(1, false);
                    self.builder.build_call(longjmp_fn, &[jmpbuf_ptr.into(), one.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unreachable()
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                Ok(true)
            }

            Stmt::Switch { expr, cases } => {
                self.compile_switch(expr, cases)
            }

            Stmt::Asm(code) => {
                self.compile_asm(code)?;
                Ok(false)
            }

            Stmt::Defer(expr) => {
                // defer: scope çıkışında LIFO sırasıyla çalıştırılır
                // defer_stack'in son frame'ine ekle
                if let Some(frame) = self.defer_stack.last_mut() {
                    frame.push(*expr.clone());
                }
                Ok(false)
            }

            Stmt::Break => {
                if let Some(&exit_bb) = self.loop_exit_bbs.last() {
                    // ARC: döngü scope'undaki class instance'ları serbest bırak
                    self.arc_release_all_scopes()?;
                    self.builder.build_unconditional_branch(exit_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }

            Stmt::Continue => {
                if let Some(&cont_bb) = self.loop_continue_bbs.last() {
                    // ARC: mevcut iterasyon scope'undaki class instance'ları serbest bırak
                    self.arc_release_all_scopes()?;
                    self.builder.build_unconditional_branch(cont_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    // ── if / else ────────────────────────────────────────────────────────────

    fn compile_if(
        &mut self,
        hint    : Option<&BranchHint>,
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

        let branch = self.builder.build_conditional_branch(cond_bool, then_bb, else_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // @Likely/@Unlikely → LLVM branch_weights metadata
        if let Some(h) = hint {
            let i32_ty = self.ctx.i32_type();
            let (then_w, else_w) = match h {
                BranchHint::Likely   => (2000u64, 1u64),
                BranchHint::Unlikely => (1u64, 2000u64),
            };
            let kind = self.ctx.get_kind_id("prof");
            let bw_str  = self.ctx.metadata_string("branch_weights");
            let tw_node = self.ctx.metadata_node(&[i32_ty.const_int(then_w, false).into()]);
            let ew_node = self.ctx.metadata_node(&[i32_ty.const_int(else_w, false).into()]);
            let prof_node = self.ctx.metadata_node(&[bw_str.into(), tw_node.into(), ew_node.into()]);
            branch.set_metadata(prof_node, kind)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

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

        self.loop_exit_bbs.push(exit_bb);
        self.loop_continue_bbs.push(cond_bb);

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

        self.loop_exit_bbs.pop();
        self.loop_continue_bbs.pop();
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

        self.loop_exit_bbs.push(exit_bb);
        self.loop_continue_bbs.push(step_bb);

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

        self.loop_exit_bbs.pop();
        self.loop_continue_bbs.pop();
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

    // ── match expression → if/else zinciri ───────────────────────────────────

    fn compile_match(
        &mut self,
        expr : &Expr,
        arms : &[MatchArm],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let cur_fn   = self.cur_fn.unwrap();
        let match_val = match self.compile_expr(expr)? {
            Some(v) => v,
            None    => return Ok(None),
        };

        // Sonuç için alloca (i64 — en geniş tip)
        let i64_ty   = self.ctx.i64_type();
        let res_alloca = self.builder.build_alloca(i64_ty, "match_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(res_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let merge_bb = self.ctx.append_basic_block(cur_fn, "match.end");

        for arm in arms {
            match &arm.pattern {
                MatchPattern::Wildcard => {
                    // Her zaman match — body derle ve sonucu kaydet
                    self.push_scope();
                    let body_val = self.compile_expr(&arm.body)?;
                    self.pop_scope();
                    if let Some(v) = body_val {
                        let stored = self.value_to_i64(v)?;
                        self.builder.build_store(res_alloca, stored)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                    self.builder.build_unconditional_branch(merge_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    break; // Wildcard her zaman son arm olmalı
                }

                MatchPattern::Variant { enum_name, variant, bindings } => {
                    // Enum variant değerini al
                    let variant_val = self.enum_variant_value(enum_name, variant)
                        .map(|v| self.ctx.i32_type().const_int(v as u64, false));

                    let then_bb = self.ctx.append_basic_block(cur_fn, "match.arm");
                    let next_bb = self.ctx.append_basic_block(cur_fn, "match.next");

                    // Karşılaştır
                    let cond = if let Some(vv) = variant_val {
                        // match_val i32'ye truncate et
                        let match_i32 = match match_val {
                            BasicValueEnum::IntValue(iv) => {
                                if iv.get_type().get_bit_width() > 32 {
                                    self.builder.build_int_truncate(iv, self.ctx.i32_type(), "mi32")
                                        .map_err(|e| CodeGenError::new(e.to_string()))?
                                } else { iv }
                            }
                            _ => self.ctx.i32_type().const_int(0, false),
                        };
                        self.builder.build_int_compare(
                            inkwell::IntPredicate::EQ, match_i32, vv, "arm_eq"
                        ).map_err(|e| CodeGenError::new(e.to_string()))?
                    } else {
                        self.ctx.bool_type().const_int(0, false)
                    };

                    self.builder.build_conditional_branch(cond, then_bb, next_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;

                    self.builder.position_at_end(then_bb);
                    self.push_scope();

                    // Bindings: şimdilik i64 olarak sabit 0 bağla (enum data henüz yok)
                    let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                    for b in bindings {
                        let alloca = self.builder.build_alloca(i64_ty, b)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_store(alloca, i64_ty.const_int(0, false))
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.define_var(b, alloca, i64_ty.into());
                    }
                    let _ = ptr_ty;

                    let body_val = self.compile_expr(&arm.body)?;
                    self.pop_scope();

                    if let Some(v) = body_val {
                        let stored = self.value_to_i64(v)?;
                        self.builder.build_store(res_alloca, stored)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                    if !self.current_block_terminated() {
                        self.builder.build_unconditional_branch(merge_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }

                    self.builder.position_at_end(next_bb);
                }
            }
        }

        // Hiç arm match etmediyse merge'e düş
        if !self.current_block_terminated() {
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.builder.position_at_end(merge_bb);
        let res = self.builder.build_load(i64_ty, res_alloca, "match_val")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(Some(res))
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

            // ── IO.print() / IO.println() ────────────────────────────────────
            Expr::StaticCall { class, method, args }
                if class == "IO" && (method == "print" || method == "println") =>
            {
                self.compile_io_print(args)?;
                if method == "println" {
                    // Newline ekle
                    let newline = self.build_global_string("\n")?;
                    let printf = self.module.get_function("printf").unwrap();
                    self.builder.build_call(printf, &[newline.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
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
                // String metod kontrolü: nesne pointer ve metod adı tanınan string metodu
                if Self::is_string_method(method) {
                    let str_val = self.compile_expr(object)?;
                    if let Some(v @ BasicValueEnum::PointerValue(_)) = str_val {
                        // Sınıf adı bilinmiyorsa string metodu olarak dene
                        if obj_class.is_none() || obj_class.as_deref() == Some("String") {
                            let args_cloned = args.to_vec();
                            return self.compile_string_method(v, method, &args_cloned);
                        }
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

                // Yerel değişkende kayıtlı lambda/fn pointer mı?
                let fn_ptr_info = self.lookup_var(class.as_str())
                    .map(|s| (s.ptr, s.ty));
                if let Some((fn_ptr_ptr, fn_ptr_ty)) = fn_ptr_info {
                    let fn_ptr_val = self.builder.build_load(fn_ptr_ty, fn_ptr_ptr, "fnptr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let is_fat  = matches!(fn_ptr_val, BasicValueEnum::StructValue(_));
                    let is_bare = matches!(fn_ptr_val, BasicValueEnum::PointerValue(_));
                    if is_fat || is_bare {
                        let i64_ty = self.ctx.i64_type();
                        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                        let mut compiled_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
                        for a in args {
                            if let Some(v) = self.compile_expr(a)? {
                                let promoted: inkwell::values::BasicMetadataValueEnum = match v {
                                    BasicValueEnum::IntValue(iv) if iv.get_type().get_bit_width() < 64 => {
                                        self.builder.build_int_z_extend(iv, i64_ty, "arg_ze")
                                            .map(|v| v.into())
                                            .unwrap_or_else(|_| iv.into())
                                    }
                                    other => other.into(),
                                };
                                compiled_args.push(promoted);
                            }
                        }
                        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr_val)?;
                        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> =
                            compiled_args.iter().map(|_| i64_ty.into()).collect();
                        if is_fat {
                            compiled_args.push(cl_ptr.into());
                            let mut fp_types = param_types;
                            fp_types.push(ptr_ty.into());
                            let fn_type = i64_ty.fn_type(&fp_types, false);
                            let call = self.builder.build_indirect_call(fn_type, fn_p, &compiled_args, "lambda_call")
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            return Ok(call.try_as_basic_value().basic());
                        } else {
                            let fn_type = i64_ty.fn_type(&param_types, false);
                            let call = self.builder.build_indirect_call(fn_type, fn_p, &compiled_args, "lambda_call")
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            return Ok(call.try_as_basic_value().basic());
                        }
                    }
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
            // Super → this pointer ile aynı, ama parent class context'inde
            // Parent method/field erişimi için 'this' pointer'ını döndür
            Expr::Super => {
                if let Some(slot) = self.lookup_var("this").cloned() {
                    let v = self.builder.build_load(slot.ty, slot.ptr, "super_val")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(v))
                } else {
                    Ok(None)
                }
            }

            // ── await ─────────────────────────────────────────────────────────
            Expr::Await(inner) => self.compile_expr(inner),

            // ── Match ─────────────────────────────────────────────────────────
            Expr::Match { expr, arms } => {
                self.compile_match(expr, arms)
            }

            // ── Lambda ───────────────────────────────────────────────────────
            Expr::Lambda { params, body } => {
                self.compile_general_lambda(params, body)
            }

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
            Expr::NullSafeAccess { object, field, args } => {
                // obj?.field veya obj?.method(args)
                // Doğru phi-node implementasyonu:
                //   null_bb  → null_ptr/0 → merge
                //   ok_bb    → gerçek değer → merge
                //   merge_bb → phi(null_val, ok_val)
                let cur_fn = match self.cur_fn { Some(f) => f, None => return Ok(None) };
                let obj_val = match self.compile_expr(object)? {
                    Some(v) => v,
                    None    => return Ok(None),
                };
                let i64_ty = self.ctx.i64_type();
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

                let ok_bb    = self.ctx.append_basic_block(cur_fn, "ns.ok");
                let merge_bb = self.ctx.append_basic_block(cur_fn, "ns.merge");

                // Null check → null_bb (= current block, falls through to merge)
                let null_src_bb = self.builder.get_insert_block().unwrap();
                let is_null = match obj_val {
                    BasicValueEnum::PointerValue(p) => {
                        let pi = self.builder.build_ptr_to_int(p, i64_ty, "ns_pi")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_int_compare(
                            inkwell::IntPredicate::EQ, pi, i64_ty.const_int(0, false), "ns_null"
                        ).map_err(|e| CodeGenError::new(e.to_string()))?
                    }
                    _ => self.ctx.bool_type().const_int(0, false),
                };
                // is_null → merge_bb (null path), !is_null → ok_bb
                self.builder.build_conditional_branch(is_null, merge_bb, ok_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                // ok_bb: gerçek dispatch
                self.builder.position_at_end(ok_bb);
                let result = match args {
                    Some(call_args) => {
                        let class_name = self.infer_object_class(object);
                        if let Some(cn) = &class_name {
                            let fn_name = format!("{}_{}", cn, field);
                            if let Some(fn_val) = self.fns.get(&fn_name).copied()
                                .or_else(|| self.module.get_function(&fn_name))
                            {
                                let mut cargs: Vec<inkwell::values::BasicMetadataValueEnum> = vec![obj_val.into()];
                                for a in call_args {
                                    if let Some(v) = self.compile_expr(a)? { cargs.push(v.into()); }
                                }
                                let call = self.builder.build_call(fn_val, &cargs, "ns_call")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                call.try_as_basic_value().basic()
                            } else { None }
                        } else { None }
                    }
                    None => self.compile_field_load(object, field)?,
                };
                let ok_val = result.unwrap_or_else(|| ptr_ty.const_null().into());
                let ok_src_bb = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                // merge_bb: phi ile birleştir
                self.builder.position_at_end(merge_bb);

                // Phi node — tip her iki daldan türet
                let merged: BasicValueEnum<'ctx> = match ok_val {
                    BasicValueEnum::PointerValue(_) => {
                        let phi = self.builder.build_phi(ptr_ty, "ns_phi")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        phi.add_incoming(&[
                            (&ok_val,                     ok_src_bb),
                            (&BasicValueEnum::from(ptr_ty.const_null()), null_src_bb),
                        ]);
                        phi.as_basic_value()
                    }
                    BasicValueEnum::IntValue(iv) => {
                        let it = iv.get_type();
                        let phi = self.builder.build_phi(it, "ns_phi")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        phi.add_incoming(&[
                            (&ok_val,                          ok_src_bb),
                            (&BasicValueEnum::from(it.const_int(0, false)), null_src_bb),
                        ]);
                        phi.as_basic_value()
                    }
                    other => other,
                };
                Ok(Some(merged))
            }
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
        self.gen_arc_list_set();
        self.gen_arc_list_filter();
        self.gen_arc_map_new();
        self.gen_arc_map_set();
        self.gen_arc_map_get_or_default();
        self.gen_arc_pair_new();
        self.gen_arc_pair_first();
        self.gen_arc_pair_second();

        if let Some(bb) = prev { self.builder.position_at_end(bb); }
    }

    fn declare_string_fns(&mut self) {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let i32_ty = self.ctx.i32_type();
        let i8_ty  = self.ctx.i8_type();

        // strlen(str) → i64
        if self.module.get_function("strlen").is_none() {
            let ft = i64_ty.fn_type(&[ptr_ty.into()], false);
            self.module.add_function("strlen", ft, None);
        }
        // strstr(haystack, needle) → ptr
        if self.module.get_function("strstr").is_none() {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strstr", ft, None);
        }
        // strncmp(a, b, n) → i32
        if self.module.get_function("strncmp").is_none() {
            let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
            self.module.add_function("strncmp", ft, None);
        }
        // strcat(dst, src) → ptr  (unsafe — dst must be big enough)
        if self.module.get_function("strcat").is_none() {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strcat", ft, None);
        }
        // strcpy(dst, src) → ptr
        if self.module.get_function("strcpy").is_none() {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strcpy", ft, None);
        }
        // toupper(c) / tolower(c) → i32
        if self.module.get_function("toupper").is_none() {
            let ft = i32_ty.fn_type(&[i32_ty.into()], false);
            self.module.add_function("toupper", ft, None);
        }
        if self.module.get_function("tolower").is_none() {
            let ft = i32_ty.fn_type(&[i32_ty.into()], false);
            self.module.add_function("tolower", ft, None);
        }
        // strtok(str, delim) → ptr
        if self.module.get_function("strtok").is_none() {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strtok", ft, None);
        }
        // strtol(str, endptr, base) → i64
        if self.module.get_function("strtol").is_none() {
            let ft = i64_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i32_ty.into()], false);
            self.module.add_function("strtol", ft, None);
        }
        // strtod(str, endptr) → f64
        if self.module.get_function("strtod").is_none() {
            let f64_ty = self.ctx.f64_type();
            let ft = f64_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strtod", ft, None);
        }
        let _ = i8_ty;
    }

    // ── String metod dispatch ─────────────────────────────────────────────────

    fn is_string_method(method: &str) -> bool {
        matches!(method, "length" | "contains" | "startsWith" | "endsWith" |
                         "compareTo" | "toUpper" | "toLower" | "trim" |
                         "split" | "indexOf" | "substring" | "replace" |
                         "parseInt" | "parseFloat" | "isEmpty" | "isBlank" |
                         "repeat" | "padStart" | "padEnd" | "chars")
    }

    fn compile_string_method(
        &mut self,
        str_val : BasicValueEnum<'ctx>,
        method  : &str,
        args    : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        self.declare_string_fns();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let i32_ty = self.ctx.i32_type();
        let str_ptr = match str_val {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Ok(None),
        };

        match method {
            // str.length() → strlen(str) : i64
            "length" => {
                let strlen = self.module.get_function("strlen").unwrap();
                let r = self.builder.build_call(strlen, &[str_ptr.into()], "slen")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            // str.contains(sub) → strstr(str, sub) != null : bool
            "contains" => {
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strstr = self.module.get_function("strstr").unwrap();
                let r = self.builder.build_call(strstr, &[str_ptr.into(), arg_ptr.into()], "found")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::PointerValue(rp)) = r.try_as_basic_value().basic() {
                    let null = ptr_ty.const_null();
                    let neq = self.builder.build_int_compare(
                        inkwell::IntPredicate::NE,
                        self.builder.build_ptr_to_int(rp, i64_ty, "p2i")
                            .map_err(|e| CodeGenError::new(e.to_string()))?,
                        self.builder.build_ptr_to_int(null, i64_ty, "np2i")
                            .map_err(|e| CodeGenError::new(e.to_string()))?,
                        "contains"
                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(neq.into()));
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            // str.startsWith(prefix) → strncmp(str, prefix, strlen(prefix)) == 0 : bool
            "startsWith" => {
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strlen = self.module.get_function("strlen").unwrap();
                let strncmp = self.module.get_function("strncmp").unwrap();
                let prefix_len = self.builder.build_call(strlen, &[arg_ptr.into()], "pfxlen")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let plen = prefix_len.try_as_basic_value().basic()
                    .ok_or_else(|| CodeGenError::new("strlen returned void"))?;
                let r = self.builder.build_call(strncmp, &[str_ptr.into(), arg_ptr.into(), plen.into()], "sw_cmp")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(cmp)) = r.try_as_basic_value().basic() {
                    let zero = i32_ty.const_int(0, false);
                    let eq = self.builder.build_int_compare(inkwell::IntPredicate::EQ, cmp, zero, "sw")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(eq.into()));
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            // str.endsWith(suffix) → strncmp(str + (len-slen), suffix, slen) == 0
            "endsWith" => {
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strlen  = self.module.get_function("strlen").unwrap();
                let strncmp = self.module.get_function("strncmp").unwrap();
                let str_len = {
                    let r = self.builder.build_call(strlen, &[str_ptr.into()], "slen")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(v)) => v,
                        _ => return Ok(Some(self.ctx.bool_type().const_int(0, false).into())),
                    }
                };
                let suf_len = {
                    let r = self.builder.build_call(strlen, &[arg_ptr.into()], "suflen")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(v)) => v,
                        _ => return Ok(Some(self.ctx.bool_type().const_int(0, false).into())),
                    }
                };
                let offset = self.builder.build_int_sub(str_len, suf_len, "ew_off")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                // GEP: str_ptr + offset
                let i8_ty  = self.ctx.i8_type();
                let start_ptr = unsafe {
                    self.builder.build_gep(i8_ty, str_ptr, &[offset], "ew_ptr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                let r = self.builder.build_call(strncmp, &[start_ptr.into(), arg_ptr.into(), suf_len.into()], "ew_cmp")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(cmp)) = r.try_as_basic_value().basic() {
                    let zero = i32_ty.const_int(0, false);
                    let eq = self.builder.build_int_compare(inkwell::IntPredicate::EQ, cmp, zero, "ew")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(eq.into()));
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            // str.compareTo(other) → strcmp(str, other) : Integer
            "compareTo" => {
                self.declare_strcmp();
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strcmp = self.module.get_function("strcmp").unwrap();
                let r = self.builder.build_call(strcmp, &[str_ptr.into(), arg_ptr.into()], "cmp")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(v)) = r.try_as_basic_value().basic() {
                    let extended = self.builder.build_int_s_extend(v, i64_ty, "cmp64")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(extended.into()));
                }
                Ok(None)
            }

            // str.toUpper() → malloc(len+1) + loop calling toupper
            "toUpper" => {
                let result = self.build_str_case_convert(str_ptr, true)?;
                Ok(Some(result.into()))
            }

            // str.toLower() → malloc(len+1) + loop calling tolower
            "toLower" => {
                let result = self.build_str_case_convert(str_ptr, false)?;
                Ok(Some(result.into()))
            }

            // str.indexOf(sub) → strstr → offset
            "indexOf" => {
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strstr = self.module.get_function("strstr").unwrap();
                let r = self.builder.build_call(strstr, &[str_ptr.into(), arg_ptr.into()], "idx_ptr")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::PointerValue(rp)) = r.try_as_basic_value().basic() {
                    let rp_int = self.builder.build_ptr_to_int(rp, i64_ty, "rp_i")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let base_int = self.builder.build_ptr_to_int(str_ptr, i64_ty, "base_i")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let null_val = i64_ty.const_int(0, false);
                    let is_null = self.builder.build_int_compare(
                        inkwell::IntPredicate::EQ, rp_int, null_val, "is_null"
                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                    let offset = self.builder.build_int_sub(rp_int, base_int, "offset")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let minus_one = i64_ty.const_int(u64::MAX, true);
                    let result = self.builder.build_select(is_null, minus_one, offset, "idx")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(result));
                }
                Ok(Some(i64_ty.const_int(u64::MAX, true).into()))
            }

            // str.split(delim) → List<String> via strtok on a copy
            "split" => {
                // malloc a copy, strtok it, push tokens into arc_list
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let result = self.build_str_split(str_ptr, arg_ptr)?;
                Ok(Some(result.into()))
            }

            // str.parseInt() → strtol(str, null, 10)
            "parseInt" => {
                let strtol  = self.module.get_function("strtol").unwrap();
                let null_ptr = ptr_ty.const_null();
                let base10   = i32_ty.const_int(10, false);
                let r = self.builder.build_call(strtol, &[str_ptr.into(), null_ptr.into(), base10.into()], "parsed_i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            // str.parseFloat() → strtod(str, null)
            "parseFloat" => {
                let strtod = self.module.get_function("strtod").unwrap();
                let null_ptr = ptr_ty.const_null();
                let r = self.builder.build_call(strtod, &[str_ptr.into(), null_ptr.into()], "parsed_f")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            // str.substring(start, end) → malloc + memcpy
            "substring" => {
                let start = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or(i64_ty.const_int(0, false));
                let end_val = args.get(1).and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or(i64_ty.const_int(0, false));
                let sub_len = self.builder.build_int_sub(end_val, start, "sub_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let sub_len1 = self.builder.build_int_add(sub_len, i64_ty.const_int(1, false), "sub_len1")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let malloc = self.module.get_function("malloc").unwrap();
                let buf_call = self.builder.build_call(malloc, &[sub_len1.into()], "sub_buf")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_ptr = match buf_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => return Ok(None),
                };
                // GEP: src_ptr + start
                let i8_ty = self.ctx.i8_type();
                let src_start = unsafe {
                    self.builder.build_gep(i8_ty, str_ptr, &[start], "sub_src")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                // memcpy(buf, src_start, sub_len)
                let memcpy_fn = self.module.get_function("memcpy").unwrap_or_else(|| {
                    let ptr = ptr_ty;
                    let ft  = ptr.fn_type(&[ptr.into(), ptr.into(), i64_ty.into()], false);
                    self.module.add_function("memcpy", ft, None)
                });
                self.builder.build_call(memcpy_fn, &[buf_ptr.into(), src_start.into(), sub_len.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                // null terminator
                let null_gep = unsafe {
                    self.builder.build_gep(i8_ty, buf_ptr, &[sub_len], "sub_null_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                self.builder.build_store(null_gep, i8_ty.const_int(0, false))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(Some(buf_ptr.into()))
            }

            // str.replace(old, new) → strstr tabanlı basit impl (ilk occurrence)
            "replace" => {
                let old_str = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::PointerValue(p) = v { Some(p) } else { None })
                    .unwrap_or(ptr_ty.const_null());
                let new_str = args.get(1).and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::PointerValue(p) = v { Some(p) } else { None })
                    .unwrap_or(ptr_ty.const_null());
                // Basit: strstr ile konumu bul, yeni string oluştur
                // malloc(strlen(src)) + strncpy(prefix) + new_str + suffix
                // Şimdilik: strstr bulamazsa src'yi döndür
                let strlen = self.module.get_function("strlen").unwrap();
                let strstr = self.module.get_function("strstr").unwrap();
                let malloc = self.module.get_function("malloc").unwrap();
                let strcpy = self.module.get_function("strcpy").unwrap();
                let strcat = self.module.get_function("strcat").unwrap();

                let found_call = self.builder.build_call(strstr, &[str_ptr.into(), old_str.into()], "rep_found")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let found_ptr = match found_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => return Ok(Some(str_ptr.into())),
                };
                // prefix_len = found - src
                let found_i  = self.builder.build_ptr_to_int(found_ptr, i64_ty, "rep_fi")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let src_i    = self.builder.build_ptr_to_int(str_ptr, i64_ty, "rep_si")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let pfx_len  = self.builder.build_int_sub(found_i, src_i, "rep_pfx")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let old_len  = {
                    let r = self.builder.build_call(strlen, &[old_str.into()], "rep_ol").map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() { Some(BasicValueEnum::IntValue(v)) => v, _ => i64_ty.const_int(0, false) }
                };
                let new_len  = {
                    let r = self.builder.build_call(strlen, &[new_str.into()], "rep_nl").map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() { Some(BasicValueEnum::IntValue(v)) => v, _ => i64_ty.const_int(0, false) }
                };
                let src_len  = {
                    let r = self.builder.build_call(strlen, &[str_ptr.into()], "rep_sl").map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() { Some(BasicValueEnum::IntValue(v)) => v, _ => i64_ty.const_int(0, false) }
                };
                // total = pfx_len + new_len + (src_len - pfx_len - old_len) + 1
                let sfx_len = self.builder.build_int_sub(src_len,
                    self.builder.build_int_add(pfx_len, old_len, "").map_err(|e| CodeGenError::new(e.to_string()))?,
                    "rep_sfxl").map_err(|e| CodeGenError::new(e.to_string()))?;
                let total = self.builder.build_int_add(
                    self.builder.build_int_add(pfx_len, new_len, "").map_err(|e| CodeGenError::new(e.to_string()))?,
                    self.builder.build_int_add(sfx_len, i64_ty.const_int(1, false), "").map_err(|e| CodeGenError::new(e.to_string()))?,
                    "rep_total"
                ).map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_call = self.builder.build_call(malloc, &[total.into()], "rep_buf")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_ptr = match buf_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => return Ok(Some(str_ptr.into())),
                };
                // Copy prefix
                let i8_ty = self.ctx.i8_type();
                let memcpy_fn = self.module.get_function("memcpy").unwrap_or_else(|| {
                    let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
                    self.module.add_function("memcpy", ft, None)
                });
                self.builder.build_call(memcpy_fn, &[buf_ptr.into(), str_ptr.into(), pfx_len.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_after_pfx = unsafe {
                    self.builder.build_gep(i8_ty, buf_ptr, &[pfx_len], "rep_apfx")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                // null terminate prefix
                self.builder.build_store(buf_after_pfx, i8_ty.const_int(0, false))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                // Append new_str + suffix
                self.builder.build_call(strcat, &[buf_ptr.into(), new_str.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let old_end = unsafe {
                    self.builder.build_gep(i8_ty, found_ptr, &[old_len], "rep_oe")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                self.builder.build_call(strcat, &[buf_ptr.into(), old_end.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(Some(buf_ptr.into()))
            }

            _ => {
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }
        }
    }

    // toUpper/toLower: malloc(len+1) + copy + loop
    fn build_str_case_convert(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        upper   : bool,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let i8_ty  = self.ctx.i8_type();
        let i32_ty = self.ctx.i32_type();

        let strlen = self.module.get_function("strlen").unwrap();
        let malloc = self.module.get_function("malloc").unwrap();
        let strcpy = self.module.get_function("strcpy").unwrap();
        let conv_fn = if upper {
            self.module.get_function("toupper").unwrap()
        } else {
            self.module.get_function("tolower").unwrap()
        };

        // len = strlen(src)
        let len_call = self.builder.build_call(strlen, &[src_ptr.into()], "cc_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(0, false),
        };
        // buf = malloc(len + 1)
        let len1 = self.builder.build_int_add(len, i64_ty.const_int(1, false), "len1")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let buf_call = self.builder.build_call(malloc, &[len1.into()], "cc_buf")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let buf_ptr = match buf_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        // strcpy(buf, src)
        self.builder.build_call(strcpy, &[buf_ptr.into(), src_ptr.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Loop: i=0; while(buf[i]) { buf[i] = conv(buf[i]); i++; }
        let cur_fn = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "cc_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "cc.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "cc.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "cc.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);

        let idx = self.builder.build_load(i64_ty, idx_alloca, "cc_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ch_ptr = unsafe {
            self.builder.build_gep(i8_ty, buf_ptr, &[idx], "cc_chp")
                .map_err(|e| CodeGenError::new(e.to_string()))?
        };
        let ch = self.builder.build_load(i8_ty, ch_ptr, "cc_ch")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ch_i64 = self.builder.build_int_z_extend(ch, i64_ty, "cc_ch64")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let not_null = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, ch_i64, i64_ty.const_int(0, false), "cc_nz"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(not_null, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let ch_i32 = self.builder.build_int_z_extend(ch, i32_ty, "cc_i32")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let conv_r = self.builder.build_call(conv_fn, &[ch_i32.into()], "cc_conv")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(BasicValueEnum::IntValue(conv_i32)) = conv_r.try_as_basic_value().basic() {
            let conv_i8 = self.builder.build_int_truncate(conv_i32, i8_ty, "cc_i8")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_store(ch_ptr, conv_i8)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }
        let next_idx = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "cc_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next_idx)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        Ok(buf_ptr)
    }

    // str.split(delim) → List<String>
    fn build_str_split(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        delim   : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();

        // list = arc_list_new()
        let list_new = self.module.get_function("arc_list_new").unwrap();
        let list_call = self.builder.build_call(list_new, &[], "split_list")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let list_ptr = match list_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };

        // Make a copy of src for strtok (strtok modifies the string)
        let strlen = self.module.get_function("strlen").unwrap();
        let malloc = self.module.get_function("malloc").unwrap();
        let strcpy = self.module.get_function("strcpy").unwrap();
        let len_r  = self.builder.build_call(strlen, &[src_ptr.into()], "sp_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_r.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(list_ptr),
        };
        let len1 = self.builder.build_int_add(len, i64_ty.const_int(1, false), "sp_len1")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let copy_call = self.builder.build_call(malloc, &[len1.into()], "sp_copy")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let copy_ptr = match copy_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(list_ptr),
        };
        self.builder.build_call(strcpy, &[copy_ptr.into(), src_ptr.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // token = strtok(copy, delim)
        let strtok = self.module.get_function("strtok").unwrap();
        let token_alloca = self.builder.build_alloca(ptr_ty, "sp_tok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let first_call = self.builder.build_call(strtok, &[copy_ptr.into(), delim.into()], "sp_first")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let first_tok = match first_call.try_as_basic_value().basic() {
            Some(v) => v,
            None => return Ok(list_ptr),
        };
        self.builder.build_store(token_alloca, first_tok)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // while (token != null) { list.append(token); token = strtok(null, delim); }
        let cur_fn   = self.cur_fn.unwrap();
        let cond_bb  = self.ctx.append_basic_block(cur_fn, "sp.cond");
        let body_bb  = self.ctx.append_basic_block(cur_fn, "sp.body");
        let exit_bb  = self.ctx.append_basic_block(cur_fn, "sp.exit");
        let list_append = self.module.get_function("arc_list_append").unwrap();
        let null_ptr = ptr_ty.const_null();

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);

        let tok = self.builder.build_load(ptr_ty, token_alloca, "sp_tok_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let tok_int = self.builder.build_ptr_to_int(
            tok.into_pointer_value(), i64_ty, "tok_i"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        let null_int = self.builder.build_ptr_to_int(null_ptr, i64_ty, "null_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let not_null = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, tok_int, null_int, "sp_nz"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(not_null, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let tok_v = self.builder.build_load(ptr_ty, token_alloca, "sp_tok_v2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_pointer_value();
        let tok_i64 = self.builder.build_ptr_to_int(tok_v, i64_ty, "tok_i64")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_call(list_append, &[list_ptr.into(), tok_i64.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let next_call = self.builder.build_call(strtok, &[null_ptr.into(), delim.into()], "sp_next")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let next_tok = match next_call.try_as_basic_value().basic() {
            Some(v) => v,
            None => ptr_ty.const_null().into(),
        };
        self.builder.build_store(token_alloca, next_tok)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        Ok(list_ptr)
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
        decl!("arc_list_set",    void, [ptr_ty.into(), i64_ty.into(), i64_ty.into()]);
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

    // ── arc_list_set(ptr list, i64 idx, i64 val) → void ────────────────────────

    fn gen_arc_list_set(&mut self) {
        let fn_name = "arc_list_set";
        if let Some(f) = self.module.get_function(fn_name) {
            if f.count_basic_blocks() > 0 { return; }
        } else {
            let i64_ty = self.ctx.i64_type();
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let fn_ty  = self.ctx.void_type().fn_type(
                &[ptr_ty.into(), i64_ty.into(), i64_ty.into()], false);
            self.module.add_function(fn_name, fn_ty, None);
        }
        let fn_val = self.module.get_function(fn_name).unwrap();
        if fn_val.count_basic_blocks() > 0 { return; }
        let i64_ty = self.ctx.i64_type();
        let entry  = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let idx      = fn_val.get_nth_param(1).unwrap().into_int_value();
        let val      = fn_val.get_nth_param(2).unwrap().into_int_value();
        let slot     = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "slot").unwrap();
        let elem_ptr = unsafe { self.builder.build_gep(i64_ty, list_ptr, &[slot], "ep").unwrap() };
        self.builder.build_store(elem_ptr, val).unwrap();
        self.builder.build_return(None).unwrap();
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

        // String metod kontrolü: 'class' isimli bir değişken ptr ise ve string metoduysa
        // (parser s.length() → StaticCall { class: "s", method: "length" } olarak parse eder)
        if Self::is_string_method(method) {
            if let Some(slot) = self.lookup_var(class).cloned() {
                if slot.class_name.is_none() {
                    let str_val = self.builder.build_load(slot.ty, slot.ptr, "str_load")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    if let BasicValueEnum::PointerValue(str_ptr) = str_val {
                        let args_cloned = args.to_vec();
                        return self.compile_string_method(str_ptr.into(), method, &args_cloned);
                    }
                }
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

        // ARC: field pointer tipinde ve class instance'ı ise release eski + retain yeni
        if matches!(field_ty, BasicTypeEnum::PointerType(_)) {
            // Field'ın Arimo class adını bul
            let field_class = self.field_arimo_types
                .get(class)
                .and_then(|m| m.get(field))
                .cloned()
                .filter(|cn| self.struct_types.contains_key(cn.as_str()));

            if let Some(ref fcn) = field_class {
                if !self.manual_memory_classes.contains(fcn.as_str()) {
                    // Eski field değerini yükle ve release et
                    if let Ok(old_val) = self.builder.build_load(field_ty, gep, "field_old") {
                        if let BasicValueEnum::PointerValue(old_ptr) = old_val {
                            let fcn_clone = fcn.clone();
                            // Geçici VarSlot ile release (alloca olarak gep kullanıyoruz — inline release)
                            let i64_ty  = self.ctx.i64_type();
                            let ptr_ty  = self.ctx.ptr_type(inkwell::AddressSpace::default());
                            let cur_fn  = self.cur_fn;
                            if let Some(cur_fn) = cur_fn {
                                if !self.current_block_terminated() {
                                    // Null check + dec + maybe free inline
                                    let dec_bb  = self.ctx.append_basic_block(cur_fn, "fs.dec");
                                    let free_bb = self.ctx.append_basic_block(cur_fn, "fs.free");
                                    let cont_bb = self.ctx.append_basic_block(cur_fn, "fs.cont");
                                    let pi = self.builder.build_ptr_to_int(old_ptr, i64_ty, "fs_pi")
                                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    let is_null = self.builder.build_int_compare(
                                        inkwell::IntPredicate::EQ, pi, i64_ty.const_int(0, false), "fs_null"
                                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                                    self.builder.build_conditional_branch(is_null, cont_bb, dec_bb)
                                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    self.builder.position_at_end(dec_bb);
                                    if let Some(&rc_idx) = self.refcount_indices.get(&fcn_clone) {
                                        if let Some(sty) = self.struct_types.get(&fcn_clone).copied() {
                                            let rc_gep = self.builder.build_struct_gep(sty, old_ptr, rc_idx, "fs_rc_gep")
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                            let rc_old = self.builder.build_load(i64_ty, rc_gep, "fs_rc")
                                                .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                                            let rc_new = self.builder.build_int_sub(rc_old, i64_ty.const_int(1, false), "fs_rcdec")
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                            self.builder.build_store(rc_gep, rc_new)
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                            let is_zero = self.builder.build_int_compare(
                                                inkwell::IntPredicate::EQ, rc_new, i64_ty.const_int(0, false), "fs_zero"
                                            ).map_err(|e| CodeGenError::new(e.to_string()))?;
                                            self.builder.build_conditional_branch(is_zero, free_bb, cont_bb)
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                        } else {
                                            self.builder.build_unconditional_branch(cont_bb)
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                        }
                                    } else {
                                        self.builder.build_unconditional_branch(cont_bb)
                                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    }
                                    self.builder.position_at_end(free_bb);
                                    let free_fn = self.module.get_function("free").unwrap_or_else(|| {
                                        let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
                                        self.module.add_function("free", ft, None)
                                    });
                                    self.builder.build_call(free_fn, &[old_ptr.into()], "")
                                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    self.builder.build_unconditional_branch(cont_bb)
                                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    self.builder.position_at_end(cont_bb);
                                }
                            }
                        }
                    }
                    // Yeni değeri retain et
                    if let BasicValueEnum::PointerValue(new_ptr) = val {
                        let fcn_clone = fcn.clone();
                        self.arc_retain_ptr(new_ptr, &fcn_clone)?;
                    }
                }
            }
        }

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
                // String concat: ptr + ptr → malloc + strcpy + strcat
                (BasicValueEnum::PointerValue(l), BasicValueEnum::PointerValue(r)) => {
                    self.declare_string_fns();
                    let i64_ty = self.ctx.i64_type();
                    let strlen = self.module.get_function("strlen").unwrap();
                    let malloc = self.module.get_function("malloc").unwrap();
                    let strcpy = self.module.get_function("strcpy").unwrap();
                    let strcat = self.module.get_function("strcat").unwrap();
                    let llen_r = self.builder.build_call(strlen, &[l.into()], "llen")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let rlen_r = self.builder.build_call(strlen, &[r.into()], "rlen")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let llen = match llen_r.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(v)) => v,
                        _ => i64_ty.const_int(0, false),
                    };
                    let rlen = match rlen_r.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(v)) => v,
                        _ => i64_ty.const_int(0, false),
                    };
                    let total = self.builder.build_int_add(llen, rlen, "concat_len")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let total1 = self.builder.build_int_add(total, i64_ty.const_int(1, false), "concat_len1")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let buf_call = self.builder.build_call(malloc, &[total1.into()], "concat_buf")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let buf_ptr = match buf_call.try_as_basic_value().basic() {
                        Some(BasicValueEnum::PointerValue(p)) => p,
                        _ => return Ok(None),
                    };
                    self.builder.build_call(strcpy, &[buf_ptr.into(), l.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_call(strcat, &[buf_ptr.into(), r.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    buf_ptr.into()
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
        // RHS'nin kaynağını belirle: Ident ise mevcut nesne kopyalanıyor → retain gerekli
        // StaticCall/MethodCall/ConstructorCall → callee +1 döndürüyor → retain YAPMA
        let rhs_needs_retain = matches!(value, Expr::Ident(_));

        let val = self.compile_expr(value)?;
        if let Some(v) = val {
            match target {
                Expr::Ident(name) => {
                    if let Some(slot) = self.lookup_var(name).cloned() {
                        let coerced = self.coerce_value(v, slot.ty)?;

                        // ARC: eski değeri her zaman release et, yeni değeri sadece Ident ise retain et
                        if let Some(ref cn) = slot.class_name.clone() {
                            if !matches!(cn.as_str(), "__List" | "__HashMap" | "__Pair") {
                                // Eski değeri release et
                                self.arc_release_var(VarSlot {
                                    ptr: slot.ptr,
                                    ty: slot.ty,
                                    class_name: Some(cn.clone()),
                                    elem_class: None,
                                })?;
                                // Yeni değeri retain et — sadece Ident RHS ise
                                if rhs_needs_retain {
                                    if let BasicValueEnum::PointerValue(new_ptr) = coerced {
                                        let cn_clone = cn.clone();
                                        self.arc_retain_ptr(new_ptr, &cn_clone)?;
                                    }
                                }
                            }
                        }

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

    // ── Koleksiyon yardımcı fonksiyonları ────────────────────────────────────

    // List iteration helper: arc_list_length + arc_list_get + arc_list_append döngüsü ile
    // her eleman için lambda(item) → yeni liste üret

    fn build_list_map(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let is_fat = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat {
            i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into()], false)
        };

        let list_len_fn    = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn    = self.module.get_function("arc_list_get").unwrap();
        let list_new_fn    = self.module.get_function("arc_list_new").unwrap();
        let list_append_fn = self.module.get_function("arc_list_append").unwrap();

        let result_call = self.builder.build_call(list_new_fn, &[], "map_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match result_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        let len_call = self.builder.build_call(list_len_fn, &[src_ptr.into()], "map_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(result_ptr),
        };

        let cur_fn = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "map_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "map.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "map.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "map.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "map_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "map_cond")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let item_call = self.builder.build_call(list_get_fn, &[src_ptr.into(), idx.into()], "map_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let item = item_call.try_as_basic_value().basic()
            .unwrap_or(i64_ty.const_int(0, false).into());
        let item_i64 = self.value_to_i64(item)?;

        let map_call_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat {
            vec![item_i64.into(), cl_ptr.into()]
        } else {
            vec![item_i64.into()]
        };
        let mapped = self.builder.build_indirect_call(fn_type, fn_p, &map_call_args, "map_call")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(mapped_val) = mapped.try_as_basic_value().basic() {
            let mapped_i64 = self.value_to_i64(mapped_val)?;
            self.builder.build_call(list_append_fn, &[result_ptr.into(), mapped_i64.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        let next = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "map_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);
        Ok(result_ptr)
    }

    // any/all: predicate'e göre boolean aggregate
    fn build_list_any_all(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
        is_any  : bool,
    ) -> CgResult<inkwell::values::IntValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let i1_ty  = self.ctx.bool_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let is_fat_aa = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p_aa, cl_ptr_aa) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat_aa {
            i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into()], false)
        };
        let list_len_fn = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn = self.module.get_function("arc_list_get").unwrap();

        let len_call = self.builder.build_call(list_len_fn, &[src_ptr.into()], "aa_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(i1_ty.const_int(if is_any { 0 } else { 1 }, false)),
        };

        let cur_fn    = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "aa_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let res_alloca = self.builder.build_alloca(i1_ty, "aa_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(res_alloca, i1_ty.const_int(if is_any { 0 } else { 1 }, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb  = self.ctx.append_basic_block(cur_fn, "aa.cond");
        let body_bb  = self.ctx.append_basic_block(cur_fn, "aa.body");
        let early_bb = self.ctx.append_basic_block(cur_fn, "aa.early");
        let exit_bb  = self.ctx.append_basic_block(cur_fn, "aa.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "aa_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "aa_cond")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let item_call = self.builder.build_call(list_get_fn, &[src_ptr.into(), idx.into()], "aa_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let item = item_call.try_as_basic_value().basic()
            .unwrap_or(i64_ty.const_int(0, false).into());
        let item_i64 = self.value_to_i64(item)?;

        let aa_call_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat_aa {
            vec![item_i64.into(), cl_ptr_aa.into()]
        } else {
            vec![item_i64.into()]
        };
        let pred_result = {
            let r = self.builder.build_indirect_call(fn_type, fn_p_aa, &aa_call_args, "aa_pred")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            r.try_as_basic_value().basic()
                .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
        };

        if let Some(pred_i64) = pred_result {
            let is_nonzero = self.builder.build_int_compare(
                inkwell::IntPredicate::NE, pred_i64, i64_ty.const_int(0, false), "aa_nz"
            ).map_err(|e| CodeGenError::new(e.to_string()))?;
            // any: if true → set result = true, break
            // all: if false → set result = false, break
            let trigger = if is_any { is_nonzero } else {
                self.builder.build_not(is_nonzero, "aa_not")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
            };
            self.builder.build_conditional_branch(trigger, early_bb, cond_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        } else {
            self.builder.build_unconditional_branch(cond_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        let next = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "aa_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        // Note: branching to cond_bb already done above, so early_bb handles early exit
        self.builder.position_at_end(early_bb);
        self.builder.build_store(res_alloca, i1_ty.const_int(if is_any { 1 } else { 0 }, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        let res = self.builder.build_load(i1_ty, res_alloca, "aa_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        Ok(res)
    }

    // reduce(init, fn) → acc
    fn build_list_reduce(
        &mut self,
        src_ptr  : inkwell::values::PointerValue<'ctx>,
        init     : inkwell::values::IntValue<'ctx>,
        fn_ptr   : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::IntValue<'ctx>> {
        let i64_ty  = self.ctx.i64_type();
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let is_fat_red = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p_red, cl_ptr_red) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat_red {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false)
        };
        let list_len_fn = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn = self.module.get_function("arc_list_get").unwrap();

        let len_call = self.builder.build_call(list_len_fn, &[src_ptr.into()], "red_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(init),
        };

        let cur_fn    = self.cur_fn.unwrap();
        let acc_alloca = self.builder.build_alloca(i64_ty, "red_acc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(acc_alloca, init)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let idx_alloca = self.builder.build_alloca(i64_ty, "red_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "red.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "red.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "red.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "red_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "red_c")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let acc = self.builder.build_load(i64_ty, acc_alloca, "red_acc_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let item_call = self.builder.build_call(list_get_fn, &[src_ptr.into(), idx.into()], "red_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let item = item_call.try_as_basic_value().basic()
            .unwrap_or(i64_ty.const_int(0, false).into());
        let item_i64 = self.value_to_i64(item)?;

        let red_call_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat_red {
            vec![acc.into(), item_i64.into(), cl_ptr_red.into()]
        } else {
            vec![acc.into(), item_i64.into()]
        };
        let new_acc = self.builder.build_indirect_call(fn_type, fn_p_red, &red_call_args, "red_call")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(BasicValueEnum::IntValue(na)) = new_acc.try_as_basic_value().basic() {
            self.builder.build_store(acc_alloca, na)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        let next = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "red_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        let result = self.builder.build_load(i64_ty, acc_alloca, "red_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        Ok(result)
    }

    // take/takeLast → ilk/son n eleman
    fn build_list_take(
        &mut self,
        src_ptr   : inkwell::values::PointerValue<'ctx>,
        n         : inkwell::values::IntValue<'ctx>,
        from_last : bool,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let list_len_fn    = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn    = self.module.get_function("arc_list_get").unwrap();
        let list_new_fn    = self.module.get_function("arc_list_new").unwrap();
        let list_append_fn = self.module.get_function("arc_list_append").unwrap();

        let result_call = self.builder.build_call(list_new_fn, &[], "take_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match result_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };

        let len_call = self.builder.build_call(list_len_fn, &[src_ptr.into()], "take_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(result_ptr),
        };

        // start = from_last ? max(0, len - n) : 0
        // end   = from_last ? len : min(n, len)
        let start = if from_last {
            let diff = self.builder.build_int_sub(len, n, "take_diff")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            let zero = i64_ty.const_int(0, false);
            let is_neg = self.builder.build_int_compare(inkwell::IntPredicate::SLT, diff, zero, "take_neg")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_select(is_neg, zero, diff, "take_start")
                .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value()
        } else {
            i64_ty.const_int(0, false)
        };

        let end = if from_last {
            len
        } else {
            let is_over = self.builder.build_int_compare(inkwell::IntPredicate::SGT, n, len, "take_over")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_select(is_over, len, n, "take_end")
                .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value()
        };

        let cur_fn = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "take_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, start)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "take.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "take.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "take.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "take_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, end, "take_cond")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let item_call = self.builder.build_call(list_get_fn, &[src_ptr.into(), idx.into()], "take_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(item) = item_call.try_as_basic_value().basic() {
            let item_i64 = self.value_to_i64(item)?;
            self.builder.build_call(list_append_fn, &[result_ptr.into(), item_i64.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }
        let next = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "take_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);
        Ok(result_ptr)
    }

    // ── sortedBy(fn) → yeni sıralı liste (bubble sort, O(n²)) ─────────────────

    fn build_list_sorted_by(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty     = self.ctx.i64_type();
        let ptr_ty     = self.ctx.ptr_type(AddressSpace::default());
        let list_new   = self.module.get_function("arc_list_new").unwrap();
        let list_app   = self.module.get_function("arc_list_append").unwrap();
        let list_get   = self.module.get_function("arc_list_length").unwrap();
        let list_getf  = self.module.get_function("arc_list_get").unwrap();
        let list_set   = self.module.get_function("arc_list_set").unwrap();

        // fat pointer ayrıştır
        let is_fat = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false)
        };

        // 1. Kaynak listeyi kopyala → result
        let res_call = self.builder.build_call(list_new, &[], "srt_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match res_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        let len_call = self.builder.build_call(list_get, &[src_ptr.into()], "srt_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(result_ptr),
        };

        let cur_fn = self.cur_fn.unwrap();

        // Tüm alloca'ları buraya — loop başlamadan önce, mevcut BB'de
        let cp_idx      = self.builder.build_alloca(i64_ty, "srt_ci")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let pass_alloca = self.builder.build_alloca(i64_ty, "srt_pass")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.build_store(cp_idx, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // 1. Kopyalama döngüsü
        let cp_cond = self.ctx.append_basic_block(cur_fn, "srt.cp.cond");
        let cp_body = self.ctx.append_basic_block(cur_fn, "srt.cp.body");
        let sort_bb = self.ctx.append_basic_block(cur_fn, "srt.outer.init");
        self.builder.build_unconditional_branch(cp_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cp_cond);
        let ci = self.builder.build_load(i64_ty, cp_idx, "srt_ci_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cp_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, ci, len, "srt_cp_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cp_ok, cp_body, sort_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cp_body);
        let elem = self.builder.build_call(list_getf, &[src_ptr.into(), ci.into()], "srt_e")
            .map_err(|e| CodeGenError::new(e.to_string()))?
            .try_as_basic_value().basic()
            .unwrap_or(i64_ty.const_int(0, false).into());
        let elem_i64 = self.value_to_i64(elem)?;
        self.builder.build_call(list_app, &[result_ptr.into(), elem_i64.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ci_next = self.builder.build_int_add(ci, i64_ty.const_int(1, false), "srt_ci_n")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(cp_idx, ci_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cp_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // 2. Bubble sort: outer loop (pass = 0..len)
        self.builder.position_at_end(sort_bb);
        self.builder.build_store(pass_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let o_cond = self.ctx.append_basic_block(cur_fn, "srt.o.cond");
        let o_body = self.ctx.append_basic_block(cur_fn, "srt.o.body");
        let srt_exit = self.ctx.append_basic_block(cur_fn, "srt.exit");
        self.builder.build_unconditional_branch(o_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_cond);
        let pass = self.builder.build_load(i64_ty, pass_alloca, "srt_pass_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let o_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, pass, len, "srt_o_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(o_ok, o_body, srt_exit)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // outer body → inner loop
        self.builder.position_at_end(o_body);
        let i_alloca = self.builder.build_alloca(i64_ty, "srt_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(i_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len_m1 = self.builder.build_int_sub(len, i64_ty.const_int(1, false), "srt_lm1")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let i_cond = self.ctx.append_basic_block(cur_fn, "srt.i.cond");
        let i_body = self.ctx.append_basic_block(cur_fn, "srt.i.body");
        let swap_bb = self.ctx.append_basic_block(cur_fn, "srt.swap");
        let no_swap = self.ctx.append_basic_block(cur_fn, "srt.noswap");
        let i_next  = self.ctx.append_basic_block(cur_fn, "srt.i.next");
        let o_next  = self.ctx.append_basic_block(cur_fn, "srt.o.next");
        self.builder.build_unconditional_branch(i_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_cond);
        let i_v = self.builder.build_load(i64_ty, i_alloca, "srt_i_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let i_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, i_v, len_m1, "srt_i_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(i_ok, i_body, o_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_body);
        let iv2 = self.builder.build_load(i64_ty, i_alloca, "srt_iv2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let i1  = self.builder.build_int_add(iv2, i64_ty.const_int(1, false), "srt_i1")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let a_call = self.builder.build_call(list_getf, &[result_ptr.into(), iv2.into()], "srt_a")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let a = a_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));
        let b_call = self.builder.build_call(list_getf, &[result_ptr.into(), i1.into()], "srt_b")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let b = b_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));
        let cmp_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat {
            vec![a.into(), b.into(), cl_ptr.into()]
        } else {
            vec![a.into(), b.into()]
        };
        let cmp_call = self.builder.build_indirect_call(fn_type, fn_p, &cmp_args, "srt_cmp")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let cmp_val = cmp_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));
        let need_swap = self.builder.build_int_compare(
            inkwell::IntPredicate::SGT, cmp_val, i64_ty.const_int(0, false), "srt_ns")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(need_swap, swap_bb, no_swap)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // swap: set(i, b), set(i+1, a)
        self.builder.position_at_end(swap_bb);
        let iv3 = self.builder.build_load(i64_ty, i_alloca, "srt_iv3")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let i1b = self.builder.build_int_add(iv3, i64_ty.const_int(1, false), "srt_i1b")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_call(list_set, &[result_ptr.into(), iv3.into(), b.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_call(list_set, &[result_ptr.into(), i1b.into(), a.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(i_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(no_swap);
        self.builder.build_unconditional_branch(i_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_next);
        let iv4 = self.builder.build_load(i64_ty, i_alloca, "srt_iv4")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let in2 = self.builder.build_int_add(iv4, i64_ty.const_int(1, false), "srt_in")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(i_alloca, in2)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(i_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_next);
        let pv2 = self.builder.build_load(i64_ty, pass_alloca, "srt_pv2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let pn  = self.builder.build_int_add(pv2, i64_ty.const_int(1, false), "srt_pn")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(pass_alloca, pn)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(o_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(srt_exit);
        Ok(result_ptr)
    }

    // ── flatMap(fn) → yeni liste (her eleman → List<T> dönüştürür, düzleştirir) ─

    fn build_list_flat_map(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty    = self.ctx.i64_type();
        let ptr_ty    = self.ctx.ptr_type(AddressSpace::default());
        let list_new  = self.module.get_function("arc_list_new").unwrap();
        let list_app  = self.module.get_function("arc_list_append").unwrap();
        let list_len  = self.module.get_function("arc_list_length").unwrap();
        let list_get  = self.module.get_function("arc_list_get").unwrap();

        let is_fat = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat {
            i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into()], false)
        };

        let res_call = self.builder.build_call(list_new, &[], "fm_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match res_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        let src_len_call = self.builder.build_call(list_len, &[src_ptr.into()], "fm_slen")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let src_len = match src_len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(result_ptr),
        };

        let cur_fn = self.cur_fn.unwrap();
        let oi = self.builder.build_alloca(i64_ty, "fm_oi")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(oi, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let o_cond = self.ctx.append_basic_block(cur_fn, "fm.o.cond");
        let o_body = self.ctx.append_basic_block(cur_fn, "fm.o.body");
        let fm_exit= self.ctx.append_basic_block(cur_fn, "fm.exit");
        self.builder.build_unconditional_branch(o_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_cond);
        let oiv = self.builder.build_load(i64_ty, oi, "fm_oiv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let o_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, oiv, src_len, "fm_o_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(o_ok, o_body, fm_exit)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_body);
        let oiv2 = self.builder.build_load(i64_ty, oi, "fm_oiv2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let elem_call = self.builder.build_call(list_get, &[src_ptr.into(), oiv2.into()], "fm_elem")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let elem_i64 = elem_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));

        // lambda(elem) → i64 (pointer to inner list)
        let lam_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat {
            vec![elem_i64.into(), cl_ptr.into()]
        } else {
            vec![elem_i64.into()]
        };
        let inner_call = self.builder.build_indirect_call(fn_type, fn_p, &lam_args, "fm_inner")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let inner_i64 = match inner_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => {
                let o_next = self.ctx.append_basic_block(cur_fn, "fm.o.next_skip");
                self.builder.build_unconditional_branch(o_next)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(o_next);
                let ov_n = self.builder.build_int_add(oiv2, i64_ty.const_int(1, false), "fm_on")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(oi, ov_n)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_unconditional_branch(o_cond)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                return Ok(result_ptr); // unreachable
            }
        };
        let inner_ptr = self.builder.build_int_to_ptr(inner_i64, ptr_ty, "fm_iptr")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let inner_len_call = self.builder.build_call(list_len, &[inner_ptr.into()], "fm_ilen")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let inner_len = match inner_len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(0, false),
        };

        // inner loop: for j = 0..inner_len: result.append(inner.get(j))
        let ji = self.builder.build_alloca(i64_ty, "fm_ji")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(ji, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let i_cond = self.ctx.append_basic_block(cur_fn, "fm.i.cond");
        let i_body = self.ctx.append_basic_block(cur_fn, "fm.i.body");
        let o_next = self.ctx.append_basic_block(cur_fn, "fm.o.next");
        self.builder.build_unconditional_branch(i_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_cond);
        let jiv = self.builder.build_load(i64_ty, ji, "fm_jiv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let j_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, jiv, inner_len, "fm_j_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(j_ok, i_body, o_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_body);
        let jiv2 = self.builder.build_load(i64_ty, ji, "fm_jiv2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ie_call = self.builder.build_call(list_get, &[inner_ptr.into(), jiv2.into()], "fm_ie")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ie_val = ie_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));
        self.builder.build_call(list_app, &[result_ptr.into(), ie_val.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let j_next = self.builder.build_int_add(jiv2, i64_ty.const_int(1, false), "fm_jn")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(ji, j_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(i_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_next);
        let o_n = self.builder.build_int_add(oiv2, i64_ty.const_int(1, false), "fm_on2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(oi, o_n)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(o_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(fm_exit);
        Ok(result_ptr)
    }

    // ── HashMap.remove(key) → bulup sil, son entryyi o slota taşı ─────────────

    fn build_map_remove(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
        key_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<()> {
        let i64_ty  = self.ctx.i64_type();
        let strcmp  = self.module.get_function("strcmp").unwrap();
        let cur_fn  = self.cur_fn.unwrap();

        // len = map[0]
        let len = self.builder.build_load(i64_ty, map_ptr, "mr_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let idx_a  = self.builder.build_alloca(i64_ty, "mr_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_a, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb  = self.ctx.append_basic_block(cur_fn, "mr.cond");
        let chk_bb   = self.ctx.append_basic_block(cur_fn, "mr.chk");
        let found_bb = self.ctx.append_basic_block(cur_fn, "mr.found");
        let next_bb  = self.ctx.append_basic_block(cur_fn, "mr.next");
        let done_bb  = self.ctx.append_basic_block(cur_fn, "mr.done");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_a, "mr_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let lt = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "mr_lt")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(lt, chk_bb, done_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(chk_bb);
        let two   = i64_ty.const_int(2, false);
        let one   = i64_ty.const_int(1, false);
        let i2    = self.builder.build_int_mul(idx, two, "mr_i2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let kslot_idx = self.builder.build_int_add(i2, one, "mr_ks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let k_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[kslot_idx], "mr_kp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let stored_ki = self.builder.build_load(i64_ty, k_gep, "mr_ski")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let stored_kp = self.builder.build_int_to_ptr(stored_ki, ptr_ty, "mr_skp")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let cmp = self.builder.build_call(strcmp, &[stored_kp.into(), key_ptr.into()], "mr_cmp")
            .map_err(|e| CodeGenError::new(e.to_string()))?
            .try_as_basic_value().basic().unwrap().into_int_value();
        let is_eq = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "mr_eq")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(is_eq, found_bb, next_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // found: copy last entry to slot i, decrement len
        self.builder.position_at_end(found_bb);
        let idx2 = self.builder.build_load(i64_ty, idx_a, "mr_i2v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let len2 = self.builder.build_load(i64_ty, map_ptr, "mr_len2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let last = self.builder.build_int_sub(len2, one, "mr_last")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        // last key slot: 1 + last*2
        let lk2  = self.builder.build_int_mul(last, two, "mr_lk2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let lks  = self.builder.build_int_add(lk2, one, "mr_lks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let lvs  = self.builder.build_int_add(lk2, two, "mr_lvs")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let lk_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[lks], "mr_lkp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let lv_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[lvs], "mr_lvp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let last_k = self.builder.build_load(i64_ty, lk_gep, "mr_lk_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let last_v = self.builder.build_load(i64_ty, lv_gep, "mr_lv_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        // target key/val slots
        let i2v  = self.builder.build_int_mul(idx2, two, "mr_i2vx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let tks  = self.builder.build_int_add(i2v, one, "mr_tks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let tvs  = self.builder.build_int_add(i2v, two, "mr_tvs")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let tk_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[tks], "mr_tkp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let tv_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[tvs], "mr_tvp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        self.builder.build_store(tk_gep, last_k)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(tv_gep, last_v)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let new_len = self.builder.build_int_sub(len2, one, "mr_nl")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(map_ptr, new_len)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(done_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(next_bb);
        let idx_n = self.builder.build_int_add(idx, one, "mr_in")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_a, idx_n)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(done_bb);
        Ok(())
    }

    // ── HashMap.forEach((k, v) → Void) ───────────────────────────────────────

    fn build_map_foreach(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
    ) -> CgResult<()> {
        let i64_ty  = self.ctx.i64_type();
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let is_fat  = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false)
        };

        let len = self.builder.build_load(i64_ty, map_ptr, "mfe_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let cur_fn = self.cur_fn.unwrap();
        let idx_a  = self.builder.build_alloca(i64_ty, "mfe_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_a, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "mfe.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "mfe.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "mfe.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_a, "mfe_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let lt  = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "mfe_lt")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(lt, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let two  = i64_ty.const_int(2, false);
        let one  = i64_ty.const_int(1, false);
        let i2   = self.builder.build_int_mul(idx, two, "mfe_i2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ks   = self.builder.build_int_add(i2, one, "mfe_ks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let vs   = self.builder.build_int_add(i2, two, "mfe_vs")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let k_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[ks], "mfe_kp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let v_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[vs], "mfe_vp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let k_val = self.builder.build_load(i64_ty, k_gep, "mfe_k")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let v_val = self.builder.build_load(i64_ty, v_gep, "mfe_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let call_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat {
            vec![k_val.into(), v_val.into(), cl_ptr.into()]
        } else {
            vec![k_val.into(), v_val.into()]
        };
        self.builder.build_indirect_call(fn_type, fn_p, &call_args, "mfe_call")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let idx_n = self.builder.build_int_add(idx, one, "mfe_in")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_a, idx_n)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        Ok(())
    }

    // distinct() → tekrar eden elemanları çıkar (O(n²) doğru implementasyon)
    // Dış döngü: her src elemanı için iç döngü: result'ta zaten var mı?
    // Yok ise ekle, var ise atla.
    fn build_list_distinct(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty     = self.ctx.i64_type();
        let ptr_ty     = self.ctx.ptr_type(AddressSpace::default());
        let len_fn     = self.module.get_function("arc_list_length").unwrap();
        let get_fn     = self.module.get_function("arc_list_get").unwrap();
        let new_fn     = self.module.get_function("arc_list_new").unwrap();
        let append_fn  = self.module.get_function("arc_list_append").unwrap();

        let res_call = self.builder.build_call(new_fn, &[], "dist_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let res_ptr = match res_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        let src_len_call = self.builder.build_call(len_fn, &[src_ptr.into()], "dist_slen")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let src_len = match src_len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(res_ptr),
        };

        let cur_fn = self.cur_fn.unwrap();

        // found_alloca: inner loop'ta duplicate bulundu mu?
        let found_alloca = self.builder.build_alloca(i64_ty, "dist_found")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // ── Outer loop: i = 0; while i < src_len ──────────────────────────
        let i_alloca = self.builder.build_alloca(i64_ty, "dist_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(i_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let outer_cond = self.ctx.append_basic_block(cur_fn, "dist.outer_cond");
        let outer_body = self.ctx.append_basic_block(cur_fn, "dist.outer_body");
        let outer_exit = self.ctx.append_basic_block(cur_fn, "dist.outer_exit");

        self.builder.build_unconditional_branch(outer_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(outer_cond);
        let i = self.builder.build_load(i64_ty, i_alloca, "dist_iv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let outer_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, i, src_len, "dist_oc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(outer_ok, outer_body, outer_exit)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(outer_body);
        let item_call = self.builder.build_call(get_fn, &[src_ptr.into(), i.into()], "dist_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let item = match item_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(0, false),
        };

        // found = 0 (not duplicate yet)
        self.builder.build_store(found_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // res_len = current length of result list
        let res_len_call = self.builder.build_call(len_fn, &[res_ptr.into()], "dist_rlen")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let res_len = match res_len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(0, false),
        };

        // ── Inner loop: j = 0; while j < res_len && !found ───────────────
        let j_alloca = self.builder.build_alloca(i64_ty, "dist_j")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(j_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let inner_cond = self.ctx.append_basic_block(cur_fn, "dist.inner_cond");
        let inner_body = self.ctx.append_basic_block(cur_fn, "dist.inner_body");
        let inner_exit = self.ctx.append_basic_block(cur_fn, "dist.inner_exit");
        let maybe_add  = self.ctx.append_basic_block(cur_fn, "dist.maybe_add");
        let outer_inc  = self.ctx.append_basic_block(cur_fn, "dist.outer_inc");

        self.builder.build_unconditional_branch(inner_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(inner_cond);
        let j = self.builder.build_load(i64_ty, j_alloca, "dist_jv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let found_v = self.builder.build_load(i64_ty, found_alloca, "dist_fv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let j_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, j, res_len, "dist_jok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let not_found = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, found_v, i64_ty.const_int(0, false), "dist_nf"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        // continue inner if j < res_len AND not_found
        let inner_ok = self.builder.build_and(j_ok, not_found, "dist_ij_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(inner_ok, inner_body, inner_exit)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(inner_body);
        let ritem_call = self.builder.build_call(get_fn, &[res_ptr.into(), j.into()], "dist_ri")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ritem = match ritem_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(u64::MAX, false),
        };
        let is_dup = self.builder.build_int_compare(inkwell::IntPredicate::EQ, ritem, item, "dist_dup")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        // if dup: found = 1
        let dup_as_i64 = self.builder.build_int_z_extend(is_dup, i64_ty, "dist_dup64")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        // found = max(found, dup_as_i64) — effectively OR
        let new_found = self.builder.build_or(found_v, dup_as_i64, "dist_new_found")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(found_alloca, new_found)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let j_next = self.builder.build_int_add(j, i64_ty.const_int(1, false), "dist_jinc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(j_alloca, j_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(inner_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // inner_exit: if not found → append item to result
        self.builder.position_at_end(inner_exit);
        self.builder.build_unconditional_branch(maybe_add)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(maybe_add);
        let found_final = self.builder.build_load(i64_ty, found_alloca, "dist_ff")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let should_add = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, found_final, i64_ty.const_int(0, false), "dist_sa"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        let do_add_bb  = self.ctx.append_basic_block(cur_fn, "dist.do_add");
        self.builder.build_conditional_branch(should_add, do_add_bb, outer_inc)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(do_add_bb);
        self.builder.build_call(append_fn, &[res_ptr.into(), item.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(outer_inc)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // outer_inc: i++
        self.builder.position_at_end(outer_inc);
        let i_next = self.builder.build_int_add(i, i64_ty.const_int(1, false), "dist_iinc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(i_alloca, i_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(outer_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(outer_exit);
        Ok(res_ptr)
    }

    // HashMap entries → List<Pair<K,V>>
    fn build_map_entries(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let i8_ty  = self.ctx.i8_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let list_new_fn    = self.module.get_function("arc_list_new").unwrap();
        let list_append_fn = self.module.get_function("arc_list_append").unwrap();
        let pair_new_fn    = self.module.get_function("arc_pair_new").unwrap();

        let result_call = self.builder.build_call(list_new_fn, &[], "ent_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match result_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };

        // map[0] = len
        let len_gep = unsafe {
            self.builder.build_gep(i8_ty, map_ptr, &[i64_ty.const_int(0, false)], "ent_lp")
                .map_err(|e| CodeGenError::new(e.to_string()))?
        };
        let len = self.builder.build_load(i64_ty, len_gep, "ent_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let cur_fn    = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "ent_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "ent.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "ent.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "ent.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "ent_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "ent_cond")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // body: key_offset = 8*(1 + i*2), val_offset = 8*(2 + i*2)
        self.builder.position_at_end(body_bb);
        let two     = i64_ty.const_int(2, false);
        let one     = i64_ty.const_int(1, false);
        let eight   = i64_ty.const_int(8, false);
        let ki_2    = self.builder.build_int_mul(idx, two, "ent_i2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let k_slot  = self.builder.build_int_add(ki_2, one, "ent_ks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let k_off   = self.builder.build_int_mul(k_slot, eight, "ent_ko")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let v_slot  = self.builder.build_int_add(ki_2, two, "ent_vs")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let v_off   = self.builder.build_int_mul(v_slot, eight, "ent_vo")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let k_gep = unsafe { self.builder.build_gep(i8_ty, map_ptr, &[k_off], "ent_kp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let v_gep = unsafe { self.builder.build_gep(i8_ty, map_ptr, &[v_off], "ent_vp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let k_val = self.builder.build_load(i64_ty, k_gep, "ent_k")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let v_val = self.builder.build_load(i64_ty, v_gep, "ent_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let pair_call = self.builder.build_call(pair_new_fn, &[k_val.into(), v_val.into()], "ent_pair")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(BasicValueEnum::PointerValue(pair_ptr)) = pair_call.try_as_basic_value().basic() {
            let pair_i64 = self.builder.build_ptr_to_int(pair_ptr, i64_ty, "ent_pi")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_call(list_append_fn, &[result_ptr.into(), pair_i64.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        let next = self.builder.build_int_add(idx, one, "ent_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);
        Ok(result_ptr)
    }

    fn build_map_keys(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        self.build_map_list(map_ptr, true)
    }

    fn build_map_values(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        self.build_map_list(map_ptr, false)
    }

    // map.keys() veya map.values() → List<T>
    // keys_mode=true → key i64 listesi, false → val i64 listesi
    fn build_map_list(
        &mut self,
        map_ptr   : inkwell::values::PointerValue<'ctx>,
        keys_mode : bool,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let list_new_fn    = self.module.get_function("arc_list_new").unwrap();
        let list_append_fn = self.module.get_function("arc_list_append").unwrap();

        let result_call = self.builder.build_call(list_new_fn, &[], "ml_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match result_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };

        // len = map[0]
        let len = self.builder.build_load(i64_ty, map_ptr, "ml_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let cur_fn     = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "ml_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "ml.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "ml.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "ml.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "ml_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "ml_c")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let two   = i64_ty.const_int(2, false);
        let one   = i64_ty.const_int(1, false);
        // key slot: 1 + i*2,  val slot: 2 + i*2
        let i2    = self.builder.build_int_mul(idx, two, "ml_i2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let off   = if keys_mode {
            self.builder.build_int_add(i2, one, "ml_ko")
                .map_err(|e| CodeGenError::new(e.to_string()))?
        } else {
            self.builder.build_int_add(i2, two, "ml_vo")
                .map_err(|e| CodeGenError::new(e.to_string()))?
        };
        let slot_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[off], "ml_gep")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let item_val = self.builder.build_load(i64_ty, slot_gep, "ml_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        self.builder.build_call(list_append_fn, &[result_ptr.into(), item_val.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let next = self.builder.build_int_add(idx, one, "ml_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);
        Ok(result_ptr)
    }

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

            ("__List", "get") => {
                let idx = args.first()
                    .and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) }
                                  else { Some(i64_ty.const_int(0, false)) })
                    .unwrap_or(i64_ty.const_int(0, false));
                let f = self.module.get_function("arc_list_get").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into(), idx.into()], "list_get")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                return Ok(r.try_as_basic_value().basic());
            }

            ("__List", "set") => {
                let idx = args.first()
                    .and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) }
                                  else { Some(i64_ty.const_int(0, false)) })
                    .unwrap_or(i64_ty.const_int(0, false));
                let val = args.get(1)
                    .and_then(|a| self.compile_expr(a).ok().flatten())
                    .map(|v| self.value_to_i64(v)).transpose()?
                    .unwrap_or(i64_ty.const_int(0, false));
                let f = self.module.get_function("arc_list_set").unwrap();
                self.builder.build_call(f, &[list_ptr.into(), idx.into(), val.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                return Ok(None);
            }

            ("__List", "removeAt") => {
                // Shift elements left from idx+1, decrement len
                let idx = args.first()
                    .and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) }
                                  else { Some(i64_ty.const_int(0, false)) })
                    .unwrap_or(i64_ty.const_int(0, false));
                let len_f = self.module.get_function("arc_list_length").unwrap();
                let get_f = self.module.get_function("arc_list_get").unwrap();
                let set_f = self.module.get_function("arc_list_set").unwrap();
                let len_call = self.builder.build_call(len_f, &[list_ptr.into()], "rm_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let len = match len_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::IntValue(v)) => v,
                    _ => return Ok(None),
                };
                // Shift: for i = idx..len-1: set(i, get(i+1))
                let cur_fn2 = self.cur_fn.unwrap();
                let rm_i   = self.builder.build_alloca(i64_ty, "rm_i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(rm_i, idx)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let lm1    = self.builder.build_int_sub(len, i64_ty.const_int(1, false), "rm_lm1")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let rm_cond = self.ctx.append_basic_block(cur_fn2, "rm.cond");
                let rm_body = self.ctx.append_basic_block(cur_fn2, "rm.body");
                let rm_exit = self.ctx.append_basic_block(cur_fn2, "rm.exit");
                self.builder.build_unconditional_branch(rm_cond)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(rm_cond);
                let ri = self.builder.build_load(i64_ty, rm_i, "rm_iv")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                let rm_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, ri, lm1, "rm_ok")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_conditional_branch(rm_ok, rm_body, rm_exit)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(rm_body);
                let ri2  = self.builder.build_load(i64_ty, rm_i, "rm_iv2")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                let ri1  = self.builder.build_int_add(ri2, i64_ty.const_int(1, false), "rm_i1")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let next_elem = self.builder.build_call(get_f, &[list_ptr.into(), ri1.into()], "rm_ne")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .try_as_basic_value().basic()
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or(i64_ty.const_int(0, false));
                self.builder.build_call(set_f, &[list_ptr.into(), ri2.into(), next_elem.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let ri_n = self.builder.build_int_add(ri2, i64_ty.const_int(1, false), "rm_in")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(rm_i, ri_n)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_unconditional_branch(rm_cond)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(rm_exit);
                // Decrement len: list[0]--
                let cur_len = self.builder.build_load(i64_ty, list_ptr, "rm_cl")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                let new_len = self.builder.build_int_sub(cur_len, i64_ty.const_int(1, false), "rm_nl")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(list_ptr, new_len)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                return Ok(None);
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

            // take / takeLast — ilk/son N elemanı döndür
            ("__List", "take") | ("__List", "takeLast") => {
                let n_val = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or_else(|| i64_ty.const_int(0, false));
                let result = self.build_list_take(list_ptr, n_val, collection == "__List" && method == "takeLast")?;
                Ok(Some(result.into()))
            }

            // map(fn) → yeni liste
            ("__List", "map") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_map(list_ptr, fn_ptr_val)?;
                        return Ok(Some(result.into()));
                    }
                }
                let f = self.module.get_function("arc_list_new").unwrap();
                let r = self.builder.build_call(f, &[], "empty").map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            // any(fn) → bool
            ("__List", "any") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_any_all(list_ptr, fn_ptr_val, true)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            // all(fn) → bool
            ("__List", "all") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_any_all(list_ptr, fn_ptr_val, false)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(self.ctx.bool_type().const_int(1, false).into()))
            }

            // reduce(init, fn) → T
            ("__List", "reduce") => {
                let init_val = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .map(|v| self.value_to_i64(v)).transpose()?
                    .unwrap_or_else(|| i64_ty.const_int(0, false));
                let lambda_expr = args.get(1);
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_reduce(list_ptr, init_val, fn_ptr_val)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(init_val.into()))
            }

            // sortedBy(fn) → yeni sıralı liste (bubble sort)
            ("__List", "sortedBy") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_sorted_by(list_ptr, fn_ptr_val)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(list_ptr.into()))
            }

            // flatMap(fn) → düzleştirilmiş liste
            ("__List", "flatMap") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_flat_map(list_ptr, fn_ptr_val)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(list_ptr.into()))
            }

            // distinct() → tekrar edenleri çıkar (basit: yeni liste, zaten olmayan ekle)
            ("__List", "distinct") => {
                let result = self.build_list_distinct(list_ptr)?;
                Ok(Some(result.into()))
            }

            // joinToString(sep) → String
            ("__List", "joinToString") => {
                for a in args { self.compile_expr(a)?; }
                let empty = self.build_global_string("")?;
                Ok(Some(empty.into()))
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
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };
                let def = i64_ty.const_int(u64::MAX, false); // sentinel: "not found"
                let f = self.module.get_function("arc_map_get_or_default").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into(), key.into(), def.into()], "ck_val")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(v)) = r.try_as_basic_value().basic() {
                    let found = self.builder.build_int_compare(
                        inkwell::IntPredicate::NE, v, i64_ty.const_int(u64::MAX, false), "ck_found"
                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(found.into()));
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            ("__HashMap", "length") => {
                // HashMap layout: [0] = len
                let i8_ty = self.ctx.i8_type();
                let gep = unsafe {
                    self.builder.build_gep(i8_ty, list_ptr,
                        &[i64_ty.const_int(0, false)], "map_len_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                // Load map length from offset 0
                let len_val = self.builder.build_load(i64_ty, gep, "map_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(Some(len_val))
            }

            ("__HashMap", "entries") => {
                let result = self.build_map_entries(list_ptr)?;
                Ok(Some(result.into()))
            }

            ("__HashMap", "keys") => {
                let result = self.build_map_keys(list_ptr)?;
                Ok(Some(result.into()))
            }

            ("__HashMap", "values") => {
                let result = self.build_map_values(list_ptr)?;
                Ok(Some(result.into()))
            }

            ("__HashMap", "remove") => {
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };
                self.build_map_remove(list_ptr, key)?;
                Ok(None)
            }

            ("__HashMap", "forEach") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        self.build_map_foreach(list_ptr, fn_ptr_val)?;
                    }
                }
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

    // ── Free variable analizi (closure capture için) ─────────────────────────

    fn collect_idents_in_expr(expr: &Expr, out: &mut Vec<String>) {
        match expr {
            Expr::Ident(n) => out.push(n.clone()),
            Expr::BinOp { left, right, .. } => {
                Self::collect_idents_in_expr(left, out);
                Self::collect_idents_in_expr(right, out);
            }
            Expr::UnaryOp { expr, .. } => Self::collect_idents_in_expr(expr, out),
            Expr::Cast { expr, .. }    => Self::collect_idents_in_expr(expr, out),
            Expr::Await(e)             => Self::collect_idents_in_expr(e, out),
            Expr::MethodCall { object, args, .. } | Expr::NullSafeAccess { object, args: Some(args), .. } => {
                Self::collect_idents_in_expr(object, out);
                for a in args { Self::collect_idents_in_expr(a, out); }
            }
            Expr::NullSafeAccess { object, args: None, .. } => Self::collect_idents_in_expr(object, out),
            Expr::StaticCall { args, .. } | Expr::ConstructorCall { args, .. } => {
                for a in args { Self::collect_idents_in_expr(a, out); }
            }
            Expr::Ternary { cond, then, else_ } => {
                Self::collect_idents_in_expr(cond, out);
                Self::collect_idents_in_expr(then, out);
                Self::collect_idents_in_expr(else_, out);
            }
            Expr::FieldAccess { object, .. } => Self::collect_idents_in_expr(object, out),
            Expr::StrInterp(parts) => {
                for p in parts {
                    if let StringPart::Interp(e) = p { Self::collect_idents_in_expr(e, out); }
                }
            }
            Expr::Lambda { params: inner_params, body } => {
                let mut inner = Vec::new();
                Self::collect_idents_in_expr(body, &mut inner);
                let inner_set: std::collections::HashSet<_> = inner_params.iter().cloned().collect();
                for n in inner { if !inner_set.contains(&n) { out.push(n); } }
            }
            Expr::Match { expr, arms } => {
                Self::collect_idents_in_expr(expr, out);
                for arm in arms { Self::collect_idents_in_expr(&arm.body, out); }
            }
            _ => {}
        }
    }

    fn find_free_vars(&self, body: &Expr, params: &[String]) -> Vec<String> {
        let mut all_idents = Vec::new();
        Self::collect_idents_in_expr(body, &mut all_idents);
        let param_set: std::collections::HashSet<_> = params.iter().cloned().collect();
        let mut free = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for name in all_idents {
            if !param_set.contains(&name) && !seen.contains(&name) && self.lookup_var(&name).is_some() {
                free.push(name.clone());
                seen.insert(name);
            }
        }
        free
    }

    // ── Genel Lambda → fat pointer { fn_ptr, closure_ptr } ──────────────────
    // Lambda her zaman (params..., ptr %closure) → i64 imzasına sahiptir.
    // Free var yoksa closure_ptr = null. Dönen değer { ptr, ptr } struct'ı.

    fn compile_general_lambda(
        &mut self,
        params : &[String],
        body   : &Expr,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        self.lambda_counter += 1;
        let counter = self.lambda_counter;
        let fn_name = format!("arc_lambda_{}", counter);

        // 1. Free variable analizi
        let free_vars = self.find_free_vars(body, params);

        // 2. Fonksiyon imzası: (params..., ptr %closure) → i64
        let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> =
            params.iter().map(|_| i64_ty.into()).collect();
        param_types.push(ptr_ty.into());
        let fn_ty  = i64_ty.fn_type(&param_types, false);
        let fn_val = self.module.add_function(&fn_name, fn_ty, None);

        // Builder durumunu kaydet
        let prev_block = self.builder.get_insert_block();
        let prev_fn    = self.cur_fn;
        let prev_class = self.cur_class.clone();

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        self.cur_fn = Some(fn_val);
        self.push_scope();

        // 3. Parametreleri scope'a ekle
        for (i, param_name) in params.iter().enumerate() {
            if let Some(pv) = fn_val.get_nth_param(i as u32) {
                let alloca = self.builder.build_alloca(i64_ty, param_name)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(alloca, pv)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.define_var(param_name, alloca, i64_ty.into());
            }
        }

        // 4. Free var'ları closure struct'tan yükle
        if !free_vars.is_empty() {
            let closure_param_idx = params.len() as u32;
            if let Some(closure_param) = fn_val.get_nth_param(closure_param_idx) {
                let closure_ptr = closure_param.into_pointer_value();
                let field_types: Vec<BasicTypeEnum<'ctx>> =
                    free_vars.iter().map(|_| i64_ty.as_basic_type_enum()).collect();
                let cty = self.ctx.struct_type(&field_types, false);
                for (i, var_name) in free_vars.iter().enumerate() {
                    let gep = self.builder.build_struct_gep(cty, closure_ptr, i as u32, "cap_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let cap_val = self.builder.build_load(i64_ty, gep, var_name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let alloca = self.builder.build_alloca(i64_ty, var_name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(alloca, cap_val)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.define_var(var_name, alloca, i64_ty.into());
                }
            }
        }

        // 5. Body derle ve i64'e çevir
        let result  = self.compile_expr(body)?;
        let ret_val = match result {
            Some(BasicValueEnum::IntValue(v)) => {
                if v.get_type().get_bit_width() < 64 {
                    self.builder.build_int_z_extend(v, i64_ty, "lz")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                } else { v }
            }
            Some(BasicValueEnum::FloatValue(f)) => {
                self.builder.build_float_to_signed_int(f, i64_ty, "f2i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
            }
            Some(BasicValueEnum::PointerValue(p)) => {
                self.builder.build_ptr_to_int(p, i64_ty, "p2i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
            }
            _ => i64_ty.const_int(0, false),
        };
        self.builder.build_return(Some(&ret_val))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.pop_scope_no_arc();
        self.cur_fn    = prev_fn;
        self.cur_class = prev_class;

        if let Some(bb) = prev_block {
            self.builder.position_at_end(bb);
        }

        // 6. Closure struct'ı heap'te oluştur ve doldur
        let closure_ptr = if !free_vars.is_empty() {
            let field_types: Vec<BasicTypeEnum<'ctx>> =
                free_vars.iter().map(|_| i64_ty.as_basic_type_enum()).collect();
            let cty     = self.ctx.struct_type(&field_types, false);
            self.declare_malloc();
            let malloc  = self.module.get_function("malloc").unwrap();
            let size    = i64_ty.const_int((free_vars.len() as u64) * 8, false);
            let alloc_call = self.builder.build_call(malloc, &[size.into()], "closure_alloc")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            let alloc_ptr = match alloc_call.try_as_basic_value().basic() {
                Some(BasicValueEnum::PointerValue(p)) => p,
                _ => return Err(CodeGenError::new("closure malloc void")),
            };
            for (i, var_name) in free_vars.iter().enumerate() {
                if let Some(slot) = self.lookup_var(var_name) {
                    let slot = slot.clone();
                    let val = self.builder.build_load(slot.ty, slot.ptr, "cap_v")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let val_i64 = self.value_to_i64(val)?;
                    let gep = self.builder.build_struct_gep(cty, alloc_ptr, i as u32, "cap_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(gep, val_i64)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
            }
            alloc_ptr
        } else {
            ptr_ty.const_null()
        };

        // 7. Fat pointer { fn_ptr, closure_ptr } döndür
        let fat_ty = self.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);
        let fat_alloca = self.builder.build_alloca(fat_ty, "fat_ptr")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let gep0 = self.builder.build_struct_gep(fat_ty, fat_alloca, 0, "fat_fn")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(gep0, fn_val.as_global_value().as_pointer_value())
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let gep1 = self.builder.build_struct_gep(fat_ty, fat_alloca, 1, "fat_cl")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(gep1, closure_ptr)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let fat = self.builder.build_load(fat_ty, fat_alloca, "fat")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        fn_val.verify(true);
        Ok(Some(fat))
    }

    // ── Fat pointer veya bare fn ptr → (fn_ptr, closure_ptr) ─────────────────

    fn extract_fn_closure(
        &mut self,
        val : BasicValueEnum<'ctx>,
    ) -> CgResult<(PointerValue<'ctx>, PointerValue<'ctx>)> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        match val {
            BasicValueEnum::PointerValue(p) => Ok((p, ptr_ty.const_null())),
            BasicValueEnum::StructValue(s)  => {
                let fn_p = self.builder.build_extract_value(s, 0, "fat_fn")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .into_pointer_value();
                let cl_p = self.builder.build_extract_value(s, 1, "fat_cl")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .into_pointer_value();
                Ok((fn_p, cl_p))
            }
            _ => Err(CodeGenError::new("expected fn ptr or fat ptr")),
        }
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
        self.emit_object_file_opts(path, false)
    }

    pub fn emit_object_file_opts(&self, path: &Path, optimize: bool) -> CgResult<()> {
        Target::initialize_x86(&InitializationConfig::default());

        let triple   = TargetMachine::get_default_triple();
        let target   = Target::from_triple(&triple)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let cpu      = TargetMachine::get_host_cpu_name();
        let features = TargetMachine::get_host_cpu_features();

        let opt_level = if optimize {
            OptimizationLevel::Aggressive
        } else {
            OptimizationLevel::Default
        };

        let machine = target.create_target_machine(
            &triple,
            cpu.to_str().unwrap_or("generic"),
            features.to_str().unwrap_or(""),
            opt_level,
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
    compile_to_object_opts(module_ast, module_name, out_path, false)
}

pub fn compile_to_object_opts(
    module_ast  : &crate::ast::Module,
    module_name : &str,
    out_path    : &Path,
    optimize    : bool,
) -> Result<(), CodeGenError> {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, module_name);
    cg.compile_module(module_ast)?;
    cg.verify_module()?;
    cg.emit_object_file_opts(out_path, optimize)
}
