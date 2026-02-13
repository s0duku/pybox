from pybox.box import PyBox

box = PyBox()
root_id = "root"
assert box.init_local(root_id)

code = """
root_val = 'I am root'
"""

box.exec(code,root_id)


child_id = "child"
assert box.init_local_from(child_id,root_id)

code = """
child_val = 'I am child'
"""

box.exec(code,child_id)


code = """
print("In root context")
print(root_val)
print(child_val)
"""

print(box.exec(code,root_id))


code = """
print("In child context")
print(root_val)
print(child_val)
"""

print(box.exec(code,child_id))
