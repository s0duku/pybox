from pybox.box import PyBox

box = PyBox()
id = "test_exec"
assert box.init_local(id)

code = """
import sys
print(sys.modules)
test_var = 1
"""

print(box.exec(code,id))

code = """
print(test_var)
"""

print(box.exec(code,id))


