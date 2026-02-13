
from pybox.box import PyBox

box = PyBox()
id = "test_stub"
assert box.init_local(id)

@box.tool
def hello_host(name):
    return f"Hello {name}"

box.exec(hello_host.stub(),id)
box.protect(id,hello_host.name)

code = """
print(hello_host('pybox'))
hello_host = "try modify"
"""
print(box.exec(code,id))