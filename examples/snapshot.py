from pybox.box import PyBox
from pybox.snapshot import PyBoxSnapshot

box = PyBox()
box.init_local("main")

box.exec("x = 100", "main")
# Create snapshot
snapshot = PyBoxSnapshot(box)

# Modify variable
box.exec("x = 999", "main")
print(box.exec("print(f'x = {x}')", "main"))

# Rollback
snapshot.restore(box)
print(box.exec("print(f'x = {x}')", "main")) 