from pybox.box import PyBox

box = PyBox()
root_id = "test_assign"
assert box.init_local(root_id)

box.assign(root_id,"test_val",{"value":"hello pybox"})

code = """
print(test_val)
"""

print(box.exec(code,root_id))