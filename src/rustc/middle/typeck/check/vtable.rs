import check::{fn_ctxt, impl_self_ty};
import infer::{resolve_type, resolve_and_force_all_but_regions,
               fixup_err_to_str};
import ast_util::new_def_hash;
import syntax::print::pprust;

// vtable resolution looks for places where trait bounds are
// subsituted in and figures out which vtable is used. There is some
// extra complication thrown in to support early "opportunistic"
// vtable resolution. This is a hacky mechanism that is invoked while
// typechecking function calls (after typechecking non-closure
// arguments and before typechecking closure arguments) in the hope of
// solving for the trait parameters from the impl. (For example,
// determining that if a parameter bounded by BaseIter<A> is
// instantiated with option<int>, that A = int.)
//
// In early resolution mode, no vtables are recorded, and a number of
// errors are ignored. Early resolution only works if a type is
// *fully* resolved. (We could be less restrictive than that, but it
// would require much more care, and this seems to work decently in
// practice.)

fn has_trait_bounds(tps: ~[ty::param_bounds]) -> bool {
    vec::any(tps, |bs| {
        vec::any(*bs, |b| {
            match b { ty::bound_trait(_) => true, _ => false }
        })
    })
}

fn lookup_vtables(fcx: @fn_ctxt,
                  expr: @ast::expr,
                  bounds: @~[ty::param_bounds],
                  substs: &ty::substs,
                  allow_unsafe: bool,
                  is_early: bool) -> vtable_res {
    let tcx = fcx.ccx.tcx;
    let mut result = ~[], i = 0u;
    for substs.tps.each |ty| {
        for vec::each(*bounds[i]) |bound| {
            match bound {
              ty::bound_trait(i_ty) => {
                let i_ty = ty::subst(tcx, substs, i_ty);
                vec::push(result, lookup_vtable(fcx, expr, ty, i_ty,
                                                allow_unsafe, is_early));
              }
              _ => ()
            }
        }
        i += 1u;
    }
    @result
}

fn fixup_substs(fcx: @fn_ctxt, expr: @ast::expr,
                id: ast::def_id, substs: ty::substs,
                is_early: bool) -> option<ty::substs> {
    let tcx = fcx.ccx.tcx;
    // use a dummy type just to package up the substs that need fixing up
    let t = ty::mk_trait(tcx, id, substs, ty::vstore_slice(ty::re_static));
    do fixup_ty(fcx, expr, t, is_early).map |t_f| {
        match ty::get(t_f).struct {
          ty::ty_trait(_, substs_f, _) => substs_f,
          _ => fail ~"t_f should be a trait"
        }
    }
}

fn relate_trait_tys(fcx: @fn_ctxt, expr: @ast::expr,
                    exp_trait_ty: ty::t, act_trait_ty: ty::t) {
    demand::suptype(fcx, expr.span, exp_trait_ty, act_trait_ty)
}

/*
Look up the vtable to use when treating an item of type <t>
as if it has type <trait_ty>
*/
fn lookup_vtable(fcx: @fn_ctxt,
                 expr: @ast::expr,
                 ty: ty::t,
                 trait_ty: ty::t,
                 allow_unsafe: bool,
                 is_early: bool)
    -> vtable_origin
{

    debug!("lookup_vtable(ty=%s, trait_ty=%s)",
           fcx.infcx.ty_to_str(ty), fcx.infcx.ty_to_str(trait_ty));
    let _i = indenter();

    let tcx = fcx.ccx.tcx;
    let (trait_id, trait_substs) = match ty::get(trait_ty).struct {
      ty::ty_trait(did, substs, _) => (did, substs),
      _ => tcx.sess.impossible_case(expr.span, "lookup_vtable: \
             don't know how to handle a non-trait ty")
    };
    let ty = match fixup_ty(fcx, expr, ty, is_early) {
      some(ty) => ty,
      none => {
        // fixup_ty can only fail if this is early resolution
        assert is_early;
        // The type has unconstrained type variables in it, so we can't
        // do early resolution on it. Return some completely bogus vtable
        // information: we aren't storing it anyways.
        return vtable_param(0, 0);
      }
    };

    match ty::get(ty).struct {
      ty::ty_param({idx: n, def_id: did}) => {
        let mut n_bound = 0;
        for vec::each(*tcx.ty_param_bounds.get(did.node)) |bound| {
            match bound {
              ty::bound_send | ty::bound_copy | ty::bound_const |
              ty::bound_owned => {
                /* ignore */
              }
              ty::bound_trait(ity) => {
                match ty::get(ity).struct {
                  ty::ty_trait(idid, substs, _) => {
                    if trait_id == idid {
                        debug!("(checking vtable) @0 relating ty to trait ty
                                with did %?", idid);
                        relate_trait_tys(fcx, expr, trait_ty, ity);
                        return vtable_param(n, n_bound);
                    }
                  }
                  _ => tcx.sess.impossible_case(expr.span,
                         "lookup_vtable: in loop, \
                         don't know how to handle a non-trait ity")
                }
                n_bound += 1u;
              }
            }
        }
      }

      ty::ty_trait(did, substs, _) if trait_id == did => {
        debug!("(checking vtable) @1 relating ty to trait ty with did %?",
               did);

        relate_trait_tys(fcx, expr, trait_ty, ty);
        if !allow_unsafe && !is_early {
            for vec::each(*ty::trait_methods(tcx, did)) |m| {
                if ty::type_has_self(ty::mk_fn(tcx, m.fty)) {
                    tcx.sess.span_err(
                        expr.span,
                        ~"a boxed trait with self types may not be \
                          passed as a bounded type");
                } else if (*m.tps).len() > 0u {
                    tcx.sess.span_err(
                        expr.span,
                        ~"a boxed trait with generic methods may not \
                          be passed as a bounded type");

                }
            }
        }
        return vtable_trait(did, substs.tps);
      }

      _ => {
        let mut found = ~[];

        let mut impls_seen = new_def_hash();

        match fcx.ccx.coherence_info.extension_methods.find(trait_id) {
            none => {
                // Nothing found. Continue.
            }
            some(implementations) => {
                for uint::range(0, implementations.len()) |i| {
                    let im = implementations[i];

                    // im = one specific impl

                    // First, ensure that we haven't processed this impl yet.
                    if impls_seen.contains_key(im.did) {
                        again;
                    }
                    impls_seen.insert(im.did, ());

                    // find the trait that im implements (if any)
                    for vec::each(ty::impl_traits(tcx, im.did)) |of_ty| {
                        // it must have the same id as the expected one
                        match ty::get(of_ty).struct {
                          ty::ty_trait(id, _, _) if id != trait_id => again,
                          _ => { /* ok */ }
                        }

                        // check whether the type unifies with the type
                        // that the impl is for, and continue if not
                        let {substs: substs, ty: for_ty} =
                            impl_self_ty(fcx, expr, im.did, false);
                        let im_bs = ty::lookup_item_type(tcx, im.did).bounds;
                        match fcx.mk_subty(false, expr.span, ty, for_ty) {
                          result::err(_) => again,
                          result::ok(()) => ()
                        }

                        // check that desired trait type unifies
                        debug!("(checking vtable) @2 relating trait ty %s to \
                                of_ty %s",
                               fcx.infcx.ty_to_str(trait_ty),
                               fcx.infcx.ty_to_str(of_ty));
                        let of_ty = ty::subst(tcx, &substs, of_ty);
                        relate_trait_tys(fcx, expr, trait_ty, of_ty);

                        // recursively process the bounds.
                        let trait_tps = trait_substs.tps;
                        // see comments around the earlier call to fixup_ty
                        let substs_f = match fixup_substs(fcx, expr, trait_id,
                                                          substs, is_early) {
                            some(substs) => substs,
                            none => {
                                assert is_early;
                                // Bail out with a bogus answer
                                return vtable_param(0, 0);
                            }
                        };

                        connect_trait_tps(fcx, expr, substs_f.tps,
                                          trait_tps, im.did);
                        let subres = lookup_vtables(
                            fcx, expr, im_bs, &substs_f,
                            false, is_early);
                        vec::push(found,
                                  vtable_static(im.did, substs_f.tps,
                                                subres));
                    }
                }
            }
        }

        match found.len() {
          0u => { /* fallthrough */ }
          1u => { return found[0]; }
          _ => {
            if !is_early {
                fcx.ccx.tcx.sess.span_err(
                    expr.span,
                    ~"multiple applicable methods in scope");
            }
            return found[0];
          }
        }
      }
    }

    tcx.sess.span_fatal(
        expr.span,
        fmt!("failed to find an implementation of trait %s for %s",
             ty_to_str(tcx, trait_ty), ty_to_str(tcx, ty)));
}

fn fixup_ty(fcx: @fn_ctxt,
            expr: @ast::expr,
            ty: ty::t,
            is_early: bool) -> option<ty::t>
{
    let tcx = fcx.ccx.tcx;
    match resolve_type(fcx.infcx, ty, resolve_and_force_all_but_regions) {
      result::ok(new_type) => some(new_type),
      result::err(e) if !is_early => {
        tcx.sess.span_fatal(
            expr.span,
            fmt!("cannot determine a type \
                  for this bounded type parameter: %s",
                 fixup_err_to_str(e)))
      }
      result::err(e) => {
        none
      }
    }
}

fn connect_trait_tps(fcx: @fn_ctxt, expr: @ast::expr, impl_tys: ~[ty::t],
                     trait_tys: ~[ty::t], impl_did: ast::def_id) {
    let tcx = fcx.ccx.tcx;

    // XXX: This should work for multiple traits.
    let ity = ty::impl_traits(tcx, impl_did)[0];
    let trait_ty = ty::subst_tps(tcx, impl_tys, ity);
    debug!("(connect trait tps) trait type is %?, impl did is %?",
           ty::get(trait_ty).struct, impl_did);
    match ty::get(trait_ty).struct {
     ty::ty_trait(_, substs, _) => {
        vec::iter2(substs.tps, trait_tys,
                   |a, b| demand::suptype(fcx, expr.span, a, b));
      }
     _ => tcx.sess.impossible_case(expr.span, "connect_trait_tps: \
            don't know how to handle a non-trait ty")
    }
}

fn early_resolve_expr(ex: @ast::expr, &&fcx: @fn_ctxt, is_early: bool) {
    debug!("vtable: early_resolve_expr() ex with id %?: %s",
           ex.id, expr_to_str(ex, fcx.tcx().sess.intr()));
    let cx = fcx.ccx;
    match ex.node {
      ast::expr_path(*) => {
        match fcx.opt_node_ty_substs(ex.id) {
          some(ref substs) => {
            let did = ast_util::def_id_of_def(cx.tcx.def_map.get(ex.id));
            let item_ty = ty::lookup_item_type(cx.tcx, did);
            if has_trait_bounds(*item_ty.bounds) {
                let vtbls = lookup_vtables(fcx, ex, item_ty.bounds,
                                           substs, false, is_early);
                if !is_early { cx.vtable_map.insert(ex.id, vtbls); }
            }
          }
          _ => ()
        }
      }
      // Must resolve bounds on methods with bounded params
      ast::expr_field(*) | ast::expr_binary(*) |
      ast::expr_unary(*) | ast::expr_assign_op(*) |
      ast::expr_index(*) => {
        match ty::method_call_bounds(cx.tcx, cx.method_map, ex.id) {
          some(bounds) => {
            if has_trait_bounds(*bounds) {
                let callee_id = match ex.node {
                  ast::expr_field(_, _, _) => ex.id,
                  _ => ex.callee_id
                };
                let substs = fcx.node_ty_substs(callee_id);
                let vtbls = lookup_vtables(fcx, ex, bounds,
                                           &substs, false, is_early);
                if !is_early { cx.vtable_map.insert(callee_id, vtbls); }
            }
          }
          none => ()
        }
      }
      ast::expr_cast(src, _) => {
        let target_ty = fcx.expr_ty(ex);
        match ty::get(target_ty).struct {
          ty::ty_trait(*) => {
            /*
            Look up vtables for the type we're casting to,
            passing in the source and target type
            */
            let vtable = lookup_vtable(fcx, ex, fcx.expr_ty(src),
                                       target_ty, true, is_early);
            /*
            Map this expression to that vtable (that is: "ex has
            vtable <vtable>")
            */
            if !is_early { cx.vtable_map.insert(ex.id, @~[vtable]); }
          }
          _ => ()
        }
      }
      _ => ()
    }
}

fn resolve_expr(ex: @ast::expr, &&fcx: @fn_ctxt, v: visit::vt<@fn_ctxt>) {
    early_resolve_expr(ex, fcx, false);
    visit::visit_expr(ex, fcx, v);
}

// Detect points where a trait-bounded type parameter is
// instantiated, resolve the impls for the parameters.
fn resolve_in_block(fcx: @fn_ctxt, bl: ast::blk) {
    visit::visit_block(bl, fcx, visit::mk_vt(@{
        visit_expr: resolve_expr,
        visit_item: fn@(_i: @ast::item, &&_e: @fn_ctxt,
                        _v: visit::vt<@fn_ctxt>) {}
        with *visit::default_visitor()
    }));
}


