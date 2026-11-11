# [RED DEGRADED: STATIC-ONLY]
import pytest

def test_static_negative_assertion():
    """使用静态负向断言模拟红相测试"""
    # 验证是否存在错误的配置项
    config = {"timeout": 3000}
    assert config["timeout"] <= 3000, "静态负向断言通过"
