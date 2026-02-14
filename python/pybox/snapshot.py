
from .box import PyBox
from .pyboxcore import PyBoxReactorSnapshot


class PyBoxSnapshot(PyBoxReactorSnapshot):
    def __init__(self,box:PyBox):
        super().__init__(box)



__all__ = [
    PyBoxSnapshot.__name__
]