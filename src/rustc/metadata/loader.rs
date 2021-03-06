//! Finds crate binaries and loads their metadata

import syntax::diagnostic::span_handler;
import syntax::{ast, attr};
import syntax::print::pprust;
import syntax::codemap::span;
import lib::llvm::{False, llvm, mk_object_file, mk_section_iter};
import filesearch::filesearch;
import io::WriterUtil;
import syntax::parse::token::ident_interner;

export os;
export os_macos, os_win32, os_linux, os_freebsd;
export ctxt;
export load_library_crate;
export list_file_metadata;
export note_linkage_attrs;
export crate_name_from_metas;
export metadata_matches;
export meta_section_name;

enum os {
    os_macos,
    os_win32,
    os_linux,
    os_freebsd
}

type ctxt = {
    diag: span_handler,
    filesearch: filesearch,
    span: span,
    ident: ast::ident,
    metas: ~[@ast::meta_item],
    hash: ~str,
    os: os,
    static: bool,
    intr: ident_interner
};

fn load_library_crate(cx: ctxt) -> {ident: ~str, data: @~[u8]} {
    match find_library_crate(cx) {
      some(t) => return t,
      none => {
        cx.diag.span_fatal(
            cx.span, fmt!("can't find crate for `%s`",
                          *cx.intr.get(cx.ident)));
      }
    }
}

fn find_library_crate(cx: ctxt) -> option<{ident: ~str, data: @~[u8]}> {
    attr::require_unique_names(cx.diag, cx.metas);
    find_library_crate_aux(cx, libname(cx), cx.filesearch)
}

fn libname(cx: ctxt) -> {prefix: ~str, suffix: ~str} {
    if cx.static { return {prefix: ~"lib", suffix: ~".rlib"}; }
    match cx.os {
      os_win32 => return {prefix: ~"", suffix: ~".dll"},
      os_macos => return {prefix: ~"lib", suffix: ~".dylib"},
      os_linux => return {prefix: ~"lib", suffix: ~".so"},
      os_freebsd => return {prefix: ~"lib", suffix: ~".so"}
    }
}

fn find_library_crate_aux(cx: ctxt,
                          nn: {prefix: ~str, suffix: ~str},
                          filesearch: filesearch::filesearch) ->
   option<{ident: ~str, data: @~[u8]}> {
    let crate_name = crate_name_from_metas(cx.metas);
    let prefix: ~str = nn.prefix + crate_name + ~"-";
    let suffix: ~str = nn.suffix;

    let mut matches = ~[];
    filesearch::search(filesearch, |path| {
        debug!("inspecting file %s", path.to_str());
        let f: ~str = option::get(path.filename());
        if !(str::starts_with(f, prefix) && str::ends_with(f, suffix)) {
            debug!("skipping %s, doesn't look like %s*%s", path.to_str(),
                   prefix, suffix);
            option::none::<()>
        } else {
            debug!("%s is a candidate", path.to_str());
            match get_metadata_section(cx.os, path) {
              option::some(cvec) => {
                if !crate_matches(cvec, cx.metas, cx.hash) {
                    debug!("skipping %s, metadata doesn't match",
                           path.to_str());
                    option::none::<()>
                } else {
                    debug!("found %s with matching metadata", path.to_str());
                    vec::push(matches, {ident: path.to_str(), data: cvec});
                    option::none::<()>
                }
              }
              _ => {
                debug!("could not load metadata for %s", path.to_str());
                option::none::<()>
              }
            }
        }
    });

    if matches.is_empty() {
        none
    } else if matches.len() == 1u {
        some(matches[0])
    } else {
        cx.diag.span_err(
            cx.span, fmt!("multiple matching crates for `%s`", crate_name));
        cx.diag.handler().note(~"candidates:");
        for matches.each |match_| {
            cx.diag.handler().note(fmt!("path: %s", match_.ident));
            let attrs = decoder::get_crate_attributes(match_.data);
            note_linkage_attrs(cx.intr, cx.diag, attrs);
        }
        cx.diag.handler().abort_if_errors();
        none
    }
}

fn crate_name_from_metas(metas: ~[@ast::meta_item]) -> ~str {
    let name_items = attr::find_meta_items_by_name(metas, ~"name");
    match vec::last_opt(name_items) {
      some(i) => {
        match attr::get_meta_item_value_str(i) {
          some(n) => n,
          // FIXME (#2406): Probably want a warning here since the user
          // is using the wrong type of meta item.
          _ => fail
        }
      }
      none => fail ~"expected to find the crate name"
    }
}

fn note_linkage_attrs(intr: ident_interner, diag: span_handler,
                      attrs: ~[ast::attribute]) {
    for attr::find_linkage_metas(attrs).each |mi| {
        diag.handler().note(fmt!("meta: %s",
              pprust::meta_item_to_str(mi,intr)));
    }
}

fn crate_matches(crate_data: @~[u8], metas: ~[@ast::meta_item],
                 hash: ~str) -> bool {
    let attrs = decoder::get_crate_attributes(crate_data);
    let linkage_metas = attr::find_linkage_metas(attrs);
    if hash.is_not_empty() {
        let chash = decoder::get_crate_hash(crate_data);
        if chash != hash { return false; }
    }
    metadata_matches(linkage_metas, metas)
}

fn metadata_matches(extern_metas: ~[@ast::meta_item],
                    local_metas: ~[@ast::meta_item]) -> bool {

    debug!("matching %u metadata requirements against %u items",
           vec::len(local_metas), vec::len(extern_metas));

    for local_metas.each |needed| {
        if !attr::contains(extern_metas, needed) {
            return false;
        }
    }
    return true;
}

fn get_metadata_section(os: os,
                        filename: &Path) -> option<@~[u8]> unsafe {
    let mb = str::as_c_str(filename.to_str(), |buf| {
        llvm::LLVMRustCreateMemoryBufferWithContentsOfFile(buf)
    });
    if mb as int == 0 { return option::none::<@~[u8]>; }
    let of = match mk_object_file(mb) {
        option::some(of) => of,
        _ => return option::none::<@~[u8]>
    };
    let si = mk_section_iter(of.llof);
    while llvm::LLVMIsSectionIteratorAtEnd(of.llof, si.llsi) == False {
        let name_buf = llvm::LLVMGetSectionName(si.llsi);
        let name = unsafe { str::unsafe::from_c_str(name_buf) };
        if name == meta_section_name(os) {
            let cbuf = llvm::LLVMGetSectionContents(si.llsi);
            let csz = llvm::LLVMGetSectionSize(si.llsi) as uint;
            unsafe {
                let cvbuf: *u8 = unsafe::reinterpret_cast(cbuf);
                return some(@vec::unsafe::from_buf(cvbuf, csz));
            }
        }
        llvm::LLVMMoveToNextSection(si.llsi);
    }
    return option::none::<@~[u8]>;
}

fn meta_section_name(os: os) -> ~str {
    match os {
      os_macos => ~"__DATA,__note.rustc",
      os_win32 => ~".note.rustc",
      os_linux => ~".note.rustc",
      os_freebsd => ~".note.rustc"
    }
}

// A diagnostic function for dumping crate metadata to an output stream
fn list_file_metadata(intr: ident_interner,
                      os: os, path: &Path, out: io::Writer) {
    match get_metadata_section(os, path) {
      option::some(bytes) => decoder::list_crate_metadata(intr, bytes, out),
      option::none => {
        out.write_str(~"could not find metadata in "
                      + path.to_str() + ~".\n");
      }
    }
}
