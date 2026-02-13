use rustpython_vm::{self, AsObject, VirtualMachine};

pub(crate) fn builtins_sanitizer(vm: &VirtualMachine) -> Result<(), String> {
    // try to delete quit and exit
    let _ = vm
        .builtins
        .as_object()
        .del_item(&*vm.ctx.new_str("threading"), vm);
    let _ = vm
        .builtins
        .as_object()
        .del_item(&*vm.ctx.new_str("_thread"), vm);
    let _ = vm
        .builtins
        .as_object()
        .del_item(&*vm.ctx.new_str("quit"), vm);
    let _ = vm
        .builtins
        .as_object()
        .del_item(&*vm.ctx.new_str("exit"), vm);
    Ok(())
}
